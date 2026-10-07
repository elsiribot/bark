
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ark_testing::{btc, lightning_test, require_bark_version, sat, Captaind, TestContext};
use ark_testing::constants::BOARD_CONFIRMATIONS;
use ark_testing::context::LightningPaymentSetup;
use ark_testing::daemon::barkd::Barkd;
use ark_testing::util::{FutureExt, poll_interval};
use ark::lightning::{Invoice, PaymentHash, Preimage};
use bark_rest_client::apis::{lightning_api, Error as ApiError};
use bark_rest_client::models::{
	LightningHoldInvoiceRequest, LightningHoldSettleRequest, LightningPayRequest,
	LightningReceiveInfo,
};
use server::database::Db;

/// Verify that lightning receives are claimed via the mailbox path by
/// running barkd in `daemon_manual_sync` mode and driving the claim with
/// explicit `POST /sync/mailbox` calls. Any other code path would be unable
/// to advance the claim in this configuration, so a successful balance
/// update is proof that the mailbox did the work.
async fn ln_receive_via_mailbox(
	ctx: &TestContext,
	_lightning: &LightningPaymentSetup,
	srv: &Captaind,
	pay: impl AsyncFn(String),
) {
	// The new ServerHtlcRecv policy breaks LN receive for bark 0.5.0 and older
	require_bark_version!(> "0.5.0");

	srv.wait_for_vtxopool(&ctx).await;
	ctx.generate_blocks(2).await;

	// barkd in manual-sync mode: no startup sync, no periodic sync, no
	// round-events subscription, no mailbox subscription. Only the server
	// connection heartbeat keeps running. Every sync has to come from a
	// REST call below.
	let barkd = ctx.barkd("barkd", srv)
		.cfg(|c| c.daemon_manual_sync = true)
		.create().await;

	// Generate two lightning invoices
	let amount_1 = sat(500_000);
	let amount_2 = sat(300_000);
	let invoice_1 = barkd.lightning_invoice(amount_1).await;
	let invoice_2 = barkd.lightning_invoice(amount_2).await;

	// In intra mode pay_lightning_wait blocks until the receiver claims.
	// The claim only lands when we pull the mailbox notification, so run
	// /sync/mailbox in a concurrent loop until the balance matches.
	let expected_balance = amount_1 + amount_2;
	tokio::join!(
		pay(invoice_1.invoice),
		pay(invoice_2.invoice),
		async {
			let mut claimed = false;
			for _ in 0..30 {
				tokio::time::sleep(Duration::from_millis(500)).await;
				barkd.sync().await;
				if barkd.bark_balance().await.spendable == expected_balance {
					claimed = true;
					break;
				}
			}
			assert!(claimed, "lightning receives should be claimed by /sync");
		},
	);

	let pending = barkd.pending_lightning_receives().await;
	assert!(pending.is_empty(), "no pending receives should remain after mailbox sync");
	assert_eq!(
		barkd.bark_balance().await.spendable, expected_balance,
		"mailbox-driven claims should reflect the full expected balance",
	);
}
lightning_test!(ln_receive_via_mailbox, |cfg| {
	cfg.invoice_check_interval = Duration::from_secs(1);
});

/// `GET /lightning/sends/{identifier}` reports `unknown` for a payment
/// hash the wallet has never seen and rejects invalid identifiers.
#[tokio::test]
async fn lightning_send_status_unknown_for_unseen_hash() {
	let ctx = TestContext::new("barkd/lightning_send_status_unknown_for_unseen_hash").await;

	let srv = ctx.captaind("server").create().await;
	let barkd = ctx.barkd("barkd1", &srv).create().await;

	let config = barkd.client_config();
	let hash = "0000000000000000000000000000000000000000000000000000000000000000";
	let info = lightning_api::get_send_status(&config, hash).await
		.expect("send status for an unseen hash should not error");
	assert_eq!(info.state, "unknown");
	assert_eq!(info.payment_hash.to_string(), hash);
	assert_eq!(info.preimage, None);

	lightning_api::get_send_status(&config, "lnbc1invalid").await
		.expect_err("an identifier that is neither a hash nor an invoice should be rejected");
}

/// Both REST payment endpoints pass `retry_for_secs` on to the server.
#[tokio::test]
async fn pay_with_retry_for() {
	// Older barkd binaries don't know the field and silently drop it.
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/pay_with_retry_for").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).funded(btc(10)).create().await;

	let barkd = ctx.barkd("barkd", &srv).funded(btc(5)).create().await;
	barkd.onchain_sync().await;
	barkd.board_amount(btc(3)).await;
	ctx.generate_blocks(BOARD_CONFIRMATIONS).await;
	barkd.sync().await;
	lightning.sync().await;

	let max = srv.config().cln_xpay_max_retry_for.as_secs();
	let config = barkd.client_config();
	let db = Db::connect(&srv.config().postgres).await.expect("connect to server db");

	// Above the maximum is capped to it.
	let pay_invoice = lightning.external.invoice(Some(btc(0.1)), "pay", "pay").await;
	let resp = lightning_api::pay(&config, LightningPayRequest {
		destination: pay_invoice,
		amount_sat: None,
		comment: None,
		retry_for_secs: Some(max + 1),
		max_fee_sat: None,
	}).await.expect("lightning pay");

	let send_invoice = lightning.external.invoice(Some(btc(0.1)), "send", "send").await;
	barkd.pay_lightning(&send_invoice, Some(max - 2)).await;
	let send_payment_hash = Invoice::from_str(&send_invoice).expect("invoice").payment_hash();

	for (payment_hash, retry_for_secs) in [
		(resp.payment_hash.expect("payment hash"), max),
		(send_payment_hash, max - 2),
	] {
		let attempt = async {
			loop {
				let attempt = db.read(async |t|
					t.get_latest_payment_attempt_by_payment_hash(payment_hash).await
				).await.expect("db read");
				if let Some(attempt) = attempt {
					break attempt;
				}
				tokio::time::sleep(poll_interval()).await;
			}
		}.wait_millis(10_000).await;
		assert_eq!(attempt.retry_for, Some(Duration::from_secs(retry_for_secs)));
	}
}

/// The HTTP status of a failed barkd call.
fn error_status<T>(err: ApiError<T>) -> u16 {
	match err {
		ApiError::ResponseError(resp) => resp.status.as_u16(),
		other => panic!("expected an error response from barkd, got: {}", other),
	}
}

async fn create_hold_invoice(
	barkd: &Barkd,
	amount_sat: u64,
	payment_hash: PaymentHash,
	expiry_secs: u64,
) -> String {
	lightning_api::generate_hold_invoice(&barkd.client_config(), LightningHoldInvoiceRequest {
		amount_sat,
		payment_hash: payment_hash.to_string(),
		description: None,
		expiry_secs,
	}).await.expect("creating hold invoice").invoice
}

async fn receive_status(barkd: &Barkd, payment_hash: PaymentHash) -> LightningReceiveInfo {
	lightning_api::get_receive_status(&barkd.client_config(), &payment_hash.to_string()).await
		.expect("getting receive status")
}

/// Poll until the receive for `payment_hash` reaches `state`.
async fn wait_for_receive_state(barkd: &Barkd, payment_hash: PaymentHash, state: &str) {
	async {
		loop {
			if receive_status(barkd, payment_hash).await.state == state {
				return;
			}
			tokio::time::sleep(poll_interval()).await;
		}
	}.wait_millis(30_000).await;
}

/// A hold receive waits for its preimage once paid, is claimed when it is
/// settled, and refuses settles and abandons that don't fit its state.
#[tokio::test]
async fn hold_receive_settle() {
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/hold_receive_settle").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal)
		.cfg(|cfg| cfg.invoice_check_interval = Duration::from_secs(1))
		.funded(btc(10)).create().await;
	srv.wait_for_vtxopool(&ctx).await;
	ctx.generate_blocks(2).await;

	let barkd = ctx.barkd("barkd", &srv).create().await;
	let config = barkd.client_config();

	let amount = sat(300_000);
	let preimage = Preimage::random();
	let payment_hash = preimage.compute_payment_hash();
	let invoice = create_hold_invoice(&barkd, amount.to_sat(), payment_hash, 600).await;
	assert_eq!(Invoice::from_str(&invoice).unwrap().payment_hash(), payment_hash);

	let info = receive_status(&barkd, payment_hash).await;
	assert!(info.hold);
	assert_eq!(info.state, "awaiting-payment");
	assert_eq!(info.invoice, invoice);
	assert_eq!(info.payment_preimage, None);

	// A second hold invoice for the same hash is refused.
	let err = lightning_api::generate_hold_invoice(&config, LightningHoldInvoiceRequest {
		amount_sat: amount.to_sat(),
		payment_hash: payment_hash.to_string(),
		description: None,
		expiry_secs: 600,
	}).await.expect_err("a second hold invoice for a hash should be refused");
	assert_eq!(error_status(err), 400);

	// Nothing to settle before the payment arrives.
	let settle = || LightningHoldSettleRequest { preimage: preimage.to_string() };
	let err = lightning_api::settle_hold_receive(&config, &payment_hash.to_string(), settle())
		.await.expect_err("settling before the payment arrived should be refused");
	assert_eq!(error_status(err), 400);

	tokio::join!(
		lightning.external.pay_bolt11(&invoice),
		async {
			wait_for_receive_state(&barkd, payment_hash, "awaiting_preimage").await;
			let info = receive_status(&barkd, payment_hash).await;
			assert!(info.hold);
			assert!(!info.htlc_vtxo_ids.is_empty(), "HTLC vtxos should be granted");
			assert_eq!(barkd.bark_balance().await.spendable, sat(0),
				"nothing is ours before the preimage is revealed");

			let wrong = LightningHoldSettleRequest { preimage: Preimage::random().to_string() };
			let err = lightning_api::settle_hold_receive(&config, &payment_hash.to_string(), wrong)
				.await.expect_err("a wrong preimage should be refused");
			assert_eq!(error_status(err), 400);

			let unknown_preimage = Preimage::random();
			let unknown = LightningHoldSettleRequest { preimage: unknown_preimage.to_string() };
			let err = lightning_api::settle_hold_receive(
				&config, &unknown_preimage.compute_payment_hash().to_string(), unknown,
			).await.expect_err("settling an unknown receive should fail");
			assert_eq!(error_status(err), 404);

			// The wrong preimage didn't change anything.
			assert_eq!(receive_status(&barkd, payment_hash).await.state, "awaiting_preimage");

			lightning_api::settle_hold_receive(&config, &payment_hash.to_string(), settle())
				.await.expect("settling with the right preimage");
		},
	);

	let info = receive_status(&barkd, payment_hash).await;
	assert_eq!(info.state, "settled");
	assert_eq!(info.payment_preimage, Some(preimage));
	assert_eq!(barkd.bark_balance().await.spendable, amount);
	assert!(barkd.pending_lightning_receives().await.is_empty());

	// Settling again is fine, abandoning isn't.
	lightning_api::settle_hold_receive(&config, &payment_hash.to_string(), settle())
		.await.expect("settling again should succeed");
	let err = lightning_api::abandon_hold_receive(&config, &payment_hash.to_string())
		.await.expect_err("abandoning a claimed receive should be refused");
	assert_eq!(error_status(err), 400);

	// The claimed funds are spendable.
	let pay_invoice = lightning.external.invoice(Some(sat(100_000)), "spend", "spend").await;
	lightning_api::pay(&config, LightningPayRequest {
		destination: pay_invoice.clone(),
		amount_sat: None,
		comment: None,
		retry_for_secs: None,
		max_fee_sat: None,
	}).await.expect("paying from the received funds");
	let pay_hash = Invoice::from_str(&pay_invoice).unwrap().payment_hash();
	async {
		loop {
			let info = lightning_api::get_send_status(&config, &pay_hash.to_string()).await
				.expect("send status");
			if info.state == "paid" {
				break;
			}
			tokio::time::sleep(poll_interval()).await;
		}
	}.wait_millis(30_000).await;
}

/// A hold receive can be abandoned while it waits for a payment, after
/// which the wallet forgets it.
#[tokio::test]
async fn hold_receive_abandon_unpaid() {
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/hold_receive_abandon_unpaid").await;
	let lightning = ctx.new_lightning_setup_no_channel("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).create().await;
	let barkd = ctx.barkd("barkd", &srv).create().await;
	let config = barkd.client_config();

	let payment_hash = Preimage::random().compute_payment_hash();
	create_hold_invoice(&barkd, 100_000, payment_hash, 600).await;

	lightning_api::abandon_hold_receive(&config, &payment_hash.to_string()).await
		.expect("abandoning an unpaid hold receive");
	assert!(barkd.pending_lightning_receives().await.is_empty());

	let err = lightning_api::abandon_hold_receive(&config, &payment_hash.to_string())
		.await.expect_err("abandoning twice should fail");
	assert_eq!(error_status(err), 404);

	// A regular receive isn't a hold receive.
	let invoice = barkd.lightning_invoice(sat(100_000)).await.invoice;
	let regular_hash = Invoice::from_str(&invoice).unwrap().payment_hash();
	let err = lightning_api::abandon_hold_receive(&config, &regular_hash.to_string())
		.await.expect_err("abandoning a regular receive should be refused");
	assert_eq!(error_status(err), 400);

	// Malformed input is refused.
	let err = lightning_api::generate_hold_invoice(&config, LightningHoldInvoiceRequest {
		amount_sat: 100_000,
		payment_hash: "not a hash".to_string(),
		description: None,
		expiry_secs: 600,
	}).await.expect_err("a malformed payment hash should be refused");
	assert_eq!(error_status(err), 400);
	let err = lightning_api::generate_hold_invoice(&config, LightningHoldInvoiceRequest {
		amount_sat: 100_000,
		payment_hash: Preimage::random().compute_payment_hash().to_string(),
		description: None,
		expiry_secs: 0,
	}).await.expect_err("a zero expiry should be refused");
	assert_eq!(error_status(err), 400);
}

/// A hold receive whose preimage doesn't arrive before its claim window
/// closes is abandoned rather than claimed.
#[tokio::test]
async fn hold_receive_abandoned_when_claim_window_closes() {
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/hold_receive_abandoned_when_claim_window_closes").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal)
		.cfg(|cfg| cfg.invoice_check_interval = Duration::from_secs(1))
		.funded(btc(10)).create().await;
	srv.wait_for_vtxopool(&ctx).await;
	ctx.generate_blocks(2).await;

	let barkd = ctx.barkd("barkd", &srv).create().await;
	let config = barkd.client_config();

	let preimage = Preimage::random();
	let payment_hash = preimage.compute_payment_hash();
	let invoice = create_hold_invoice(&barkd, 300_000, payment_hash, 600).await;

	// The payer stays pending: the HTLC is never settled.
	let payment = lightning.external.try_pay_bolt11(&invoice);
	tokio::pin!(payment);
	tokio::select! {
		res = &mut payment => panic!("payment should not complete: {:?}", res),
		_ = wait_for_receive_state(&barkd, payment_hash, "awaiting_preimage") => {},
	}

	// The test server grants max_user_invoice_cltv_delta (58) blocks, the
	// claim needs vtxo_exit_delta (12) + htlc_expiry_delta (6) +
	// vtxo_exit_margin (12) + htlc_recv_claim_delta (18) = 48 of them, which
	// leaves a window of 10 blocks.
	ctx.generate_blocks(11).await;
	async {
		loop {
			barkd.sync().await;
			if barkd.pending_lightning_receives().await.is_empty() {
				return;
			}
			tokio::time::sleep(poll_interval()).await;
		}
	}.wait_millis(30_000).await;

	let settle = LightningHoldSettleRequest { preimage: preimage.to_string() };
	let err = lightning_api::settle_hold_receive(&config, &payment_hash.to_string(), settle)
		.await.expect_err("settling an abandoned receive should fail");
	assert_eq!(error_status(err), 404);
	assert_eq!(barkd.bark_balance().await.spendable, sat(0));
}

/// A hold receive paid after its expiry is not claimed, and the payer is
/// refunded once the server gives up on the held HTLC.
#[tokio::test]
async fn hold_receive_not_claimed_after_expiry() {
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/hold_receive_not_claimed_after_expiry").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal)
		.cfg(|cfg| cfg.invoice_check_interval = Duration::from_secs(1))
		.funded(btc(10)).create().await;
	srv.wait_for_vtxopool(&ctx).await;
	ctx.generate_blocks(2).await;

	let barkd = ctx.barkd("barkd", &srv).create().await;

	let payment_hash = Preimage::random().compute_payment_hash();
	let invoice = create_hold_invoice(&barkd, 300_000, payment_hash, 1).await;
	tokio::time::sleep(Duration::from_secs(2)).await;

	// The server fails the HTLC back after its forward timeout (30s in tests).
	let (res, _) = tokio::join!(
		lightning.external.try_pay_bolt11(&invoice).wait_millis(90_000),
		async {
			async {
				loop {
					barkd.sync().await;
					if barkd.pending_lightning_receives().await.is_empty() {
						return;
					}
					tokio::time::sleep(poll_interval()).await;
				}
			}.wait_millis(60_000).await;
		},
	);
	res.expect_err("a payment after the hold receive's expiry should fail");
	assert_eq!(barkd.bark_balance().await.spendable, sat(0));
}

/// A failed payment is reported as failed, keeps that state, and can't be
/// retried through barkd. A fee above `max_fee_sat` is refused upfront.
#[tokio::test]
async fn lightning_send_failure_is_final() {
	require_bark_version!(> "0.7.1");

	let ctx = TestContext::new("barkd/lightning_send_failure_is_final").await;
	// No channel: every payment fails.
	let lightning = ctx.new_lightning_setup_no_channel("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal)
		.cfg(|cfg| cfg.fees.lightning_send.base_fee = sat(500))
		.funded(btc(10)).create().await;
	srv.wait_for_vtxopool(&ctx).await;

	let barkd = ctx.barkd("barkd", &srv).funded(btc(5)).create().await;
	barkd.onchain_sync().await;
	barkd.board_amount(btc(1)).await;
	ctx.generate_blocks(BOARD_CONFIRMATIONS).await;
	barkd.wait_for_boards_synced().await;
	let config = barkd.client_config();

	let invoice = lightning.external.invoice(Some(sat(100_000)), "fail", "fail").await;
	let payment_hash = Invoice::from_str(&invoice).unwrap().payment_hash();
	let request = |max_fee_sat| LightningPayRequest {
		destination: invoice.clone(),
		amount_sat: None,
		comment: None,
		retry_for_secs: None,
		max_fee_sat,
	};

	// The fee is 500 sat, so a cap below is refused before anything happens.
	assert!(lightning_api::pay(&config, request(Some(499))).await.is_err(),
		"a fee above max_fee_sat should be refused");
	let info = lightning_api::get_send_status(&config, &payment_hash.to_string()).await.unwrap();
	assert_eq!(info.state, "unknown");
	assert_eq!(barkd.bark_balance().await.spendable, btc(1));

	lightning_api::pay(&config, request(Some(500))).await.expect("starting the payment");
	let info = lightning_api::get_send_status(&config, &payment_hash.to_string()).await.unwrap();
	assert_ne!(info.state, "unknown", "the payment should be known right after it started");

	let info = async {
		loop {
			let info = lightning_api::get_send_status(&config, &payment_hash.to_string()).await
				.expect("send status");
			if info.state == "failed" {
				return info;
			}
			assert_ne!(info.state, "paid");
			barkd.sync().await;
			tokio::time::sleep(poll_interval()).await;
		}
	}.wait_millis(120_000).await;
	assert!(info.failure_reason.is_some());
	assert_eq!(info.preimage, None);
	// The HTLCs were revoked, refunding amount and fee.
	assert_eq!(barkd.bark_balance().await.spendable, btc(1));

	assert!(lightning_api::pay(&config, request(None)).await.is_err(),
		"paying a failed invoice again should be refused");
	let info = lightning_api::get_send_status(&config, &payment_hash.to_string()).await.unwrap();
	assert_eq!(info.state, "failed");
}
