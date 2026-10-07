use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use bitcoin::Amount;
use anyhow::Context;
use utoipa::OpenApi;

use ark::lightning::{Offer, PaymentHash, Preimage};
use bark::LightningSendOptions;
use bark::actions::lightning::receive::HoldReceiveError;
use bark::lightning_invoice::{Bolt11Invoice, Description};
use bark::lnurllib::lightning_address::LightningAddress;
use bark::lnurllib::lnurl::LnUrl;

use crate::error::{self, badarg, ContextExt, ErrorResponse, HandlerResult};
use crate::ServerState;

#[derive(OpenApi)]
#[openapi(
	paths(
		generate_invoice,
		generate_invoice_for_address,
		generate_hold_invoice,
		settle_hold_receive,
		abandon_hold_receive,
		get_receive_status,
		list_receive_statuses,
		cancel_receive,
		pay,
		get_send_status,
	),
	components(schemas(
		bark_json::web::LightningInvoiceRequest,
		bark_json::web::LightningInvoiceForAddressRequest,
		bark_json::web::LightningHoldInvoiceRequest,
		bark_json::web::LightningHoldSettleRequest,
		bark_json::web::EmptyResponse,
		bark_json::cli::InvoiceInfo,
		bark_json::cli::LightningReceiveInfo,
		bark_json::web::LightningPayRequest,
		bark_json::web::LightningPayResponse,
		bark_json::cli::LightningSendInfo,
	)),
	tags((name = "lightning", description = "Create Lightning invoices and track receives."))
)]
pub struct LightningApiDoc;

pub fn router() -> Router<Arc<ServerState>> {
	Router::new()
		.route("/receives/invoice", post(generate_invoice))
		.route("/receives/invoice/for-address", post(generate_invoice_for_address))
		.route("/receives/hold-invoice", post(generate_hold_invoice))
		// The router needs one name per path segment, so these say
		// `identifier` like their siblings although they take a payment hash.
		.route("/receives/{identifier}/settle", post(settle_hold_receive))
		.route("/receives/{identifier}/abandon", post(abandon_hold_receive))
		.route("/receives/{identifier}", get(get_receive_status).delete(cancel_receive))
		.route("/receives", get(list_receive_statuses))
		.route("/sends/{identifier}", get(get_send_status))
		.route("/pay", post(pay))
}

#[utoipa::path(
	post,
	path = "/receives/invoice",
	summary = "Create a BOLT11 invoice",
	request_body = bark_json::web::LightningInvoiceRequest,
	responses(
		(status = 200, description = "Returns the created invoice", body = bark_json::cli::InvoiceInfo),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Generates a new BOLT11 invoice for the specified amount via the Ark server, \
		creating a pending Lightning receive.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn generate_invoice(
	State(state): State<Arc<ServerState>>,
	Json(body): Json<bark_json::web::LightningInvoiceRequest>,
) -> HandlerResult<Json<bark_json::cli::InvoiceInfo>> {
	let wallet = state.require_wallet()?;

	let amount = Amount::from_sat(body.amount_sat);
	let invoice = wallet.bolt11_invoice(amount, body.description, body.token).await
		.context("Failed to create invoice")?;

	Ok(axum::Json(bark_json::cli::InvoiceInfo {
		invoice: invoice.to_string(),
	}))
}

#[utoipa::path(
	post,
	path = "/receives/invoice/for-address",
	summary = "Create a BOLT11 invoice for an Ark address",
	request_body = bark_json::web::LightningInvoiceForAddressRequest,
	responses(
		(status = 200, description = "Returns the created invoice", body = bark_json::cli::InvoiceInfo),
		(status = 400, description = "Bad request", body = error::BadRequestError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Generates a new BOLT11 invoice. When paid, the wallet claims the Lightning \
		receive and forwards the resulting Ark VTXO to the supplied Ark address mailbox.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn generate_invoice_for_address(
	State(state): State<Arc<ServerState>>,
	Json(body): Json<bark_json::web::LightningInvoiceForAddressRequest>,
) -> HandlerResult<Json<bark_json::cli::InvoiceInfo>> {
	let wallet = state.require_wallet()?;

	let amount = Amount::from_sat(body.amount_sat);
	let address = ark::Address::from_str(&body.address)
		.badarg("address is not a valid Ark address")?;
	wallet.validate_arkoor_address(&address).await
		.badarg("invalid arkoor address")?;
	let invoice = wallet.bolt11_invoice_for_address(amount, address, body.description, None).await
		.context("Failed to create invoice for address")?;

	Ok(axum::Json(bark_json::cli::InvoiceInfo {
		invoice: invoice.to_string(),
	}))
}

/// Turn the error of a hold receive method into a response: refusals are the
/// caller's fault, anything else is ours.
fn hold_receive_error(error: anyhow::Error) -> ErrorResponse {
	match error.downcast_ref::<HoldReceiveError>() {
		Some(HoldReceiveError::NotFound(payment_hash)) => {
			ErrorResponse::NotFound(error::NotFoundError {
				resources: vec![payment_hash.to_string()],
				message: format!("{:#}", error),
			})
		},
		Some(_) => ErrorResponse::BadRequest(error::BadRequestError {
			message: format!("{:#}", error),
		}),
		None => ErrorResponse::from(error),
	}
}

#[utoipa::path(
	post,
	path = "/receives/hold-invoice",
	summary = "Create a BOLT11 hold invoice",
	request_body = bark_json::web::LightningHoldInvoiceRequest,
	responses(
		(status = 200, description = "Returns the created invoice", body = bark_json::cli::InvoiceInfo),
		(status = 400, description = "Bad request", body = error::BadRequestError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Generates a BOLT11 invoice for a payment hash whose preimage the caller \
		holds, creating a pending hold receive. Once the payment arrives and the Ark server has \
		granted the HTLC-recv VTXOs, the receive's state becomes `awaiting_preimage` and it waits \
		for the caller to settle it with the preimage or abandon it. A payment arriving after \
		`expiry_secs` is not claimed. Fails if the wallet already has a receive for the payment \
		hash.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn generate_hold_invoice(
	State(state): State<Arc<ServerState>>,
	Json(body): Json<bark_json::web::LightningHoldInvoiceRequest>,
) -> HandlerResult<Json<bark_json::cli::InvoiceInfo>> {
	let wallet = state.require_wallet()?;

	if body.amount_sat == 0 {
		badarg!("amount_sat must be greater than zero");
	}
	if body.expiry_secs == 0 {
		badarg!("expiry_secs must be greater than zero");
	}
	let payment_hash = PaymentHash::from_str(&body.payment_hash)
		.badarg("payment_hash is not a valid hex-encoded 32-byte hash")?;
	if let Some(description) = body.description.as_ref() {
		Description::new(description.clone()).badarg("invalid invoice description")?;
	}

	let amount = Amount::from_sat(body.amount_sat);
	let expiry = Duration::from_secs(body.expiry_secs);
	let invoice = wallet.bolt11_hold_invoice(amount, payment_hash, body.description, expiry).await
		.context("Failed to create hold invoice")
		.map_err(hold_receive_error)?;

	Ok(axum::Json(bark_json::cli::InvoiceInfo {
		invoice: invoice.to_string(),
	}))
}

#[utoipa::path(
	post,
	path = "/receives/{payment_hash}/settle",
	summary = "Settle a hold receive",
	params(
		("payment_hash" = String, Path, description = "Payment hash of the hold receive"),
	),
	request_body = bark_json::web::LightningHoldSettleRequest,
	responses(
		(status = 200, description = "The payment was claimed", body = bark_json::web::EmptyResponse),
		(status = 400, description = "The preimage doesn't match the payment hash, the receive \
			is not a hold receive, or it doesn't await a preimage", body = error::BadRequestError),
		(status = 404, description = "No receive for this payment hash", body = error::NotFoundError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Supplies the preimage of a hold receive in the `awaiting_preimage` state and \
		claims the payment. The preimage is stored before it is revealed, so after an internal \
		error the call can be repeated, and the wallet also finishes the claim on its own. \
		Repeating the call after the payment was claimed succeeds.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn settle_hold_receive(
	State(state): State<Arc<ServerState>>,
	Path(payment_hash): Path<String>,
	Json(body): Json<bark_json::web::LightningHoldSettleRequest>,
) -> HandlerResult<Json<bark_json::web::EmptyResponse>> {
	let wallet = state.require_wallet()?;

	let payment_hash = PaymentHash::from_str(&payment_hash)
		.badarg("payment hash is not a valid hex-encoded 32-byte hash")?;
	let preimage = Preimage::from_str(&body.preimage)
		.badarg("preimage is not a valid hex-encoded 32-byte preimage")?;

	wallet.settle_lightning_hold_receive(payment_hash, preimage).await
		.context("Failed to settle hold receive")
		.map_err(hold_receive_error)?;

	Ok(axum::Json(bark_json::web::EmptyResponse {}))
}

#[utoipa::path(
	post,
	path = "/receives/{payment_hash}/abandon",
	summary = "Abandon a hold receive",
	params(
		("payment_hash" = String, Path, description = "Payment hash of the hold receive"),
	),
	responses(
		(status = 200, description = "The receive was abandoned", body = bark_json::web::EmptyResponse),
		(status = 400, description = "The receive is not a hold receive, or its preimage was \
			already supplied", body = error::BadRequestError),
		(status = 404, description = "No receive for this payment hash", body = error::NotFoundError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Gives up on a hold receive whose preimage has not been supplied. Before the \
		payment arrived this cancels the receive. Once the HTLC-recv VTXOs are granted they are \
		dropped unclaimed, and the Ark server fails the inbound payment back to the sender when \
		the invoice expires.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn abandon_hold_receive(
	State(state): State<Arc<ServerState>>,
	Path(payment_hash): Path<String>,
) -> HandlerResult<Json<bark_json::web::EmptyResponse>> {
	let wallet = state.require_wallet()?;

	let payment_hash = PaymentHash::from_str(&payment_hash)
		.badarg("payment hash is not a valid hex-encoded 32-byte hash")?;

	wallet.abandon_lightning_hold_receive(payment_hash).await
		.context("Failed to abandon hold receive")
		.map_err(hold_receive_error)?;

	Ok(axum::Json(bark_json::web::EmptyResponse {}))
}

#[utoipa::path(
	get,
	path = "/receives/{identifier}",
	summary = "Get receive status",
	params(
		("identifier" = String, Path, description = "Payment hash, invoice string or preimage to search for"),
	),
	responses(
		(status = 200, description = "Returns the Lightning receive status", body = bark_json::cli::LightningReceiveInfo),
		(status = 400, description = "Bad request", body = error::BadRequestError),
		(status = 404, description = "Not found", body = error::NotFoundError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Returns the status of a specified Lightning receive, identified by its \
		payment hash, invoice string, or preimage. The response tracks progress through \
		timestamps: `htlc_vtxos` is populated once HTLCs are created by the Ark server, \
		`preimage_revealed_at` records when the preimage was sent, and `finished_at` \
		indicates the receive has settled or been canceled.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn get_receive_status(
	State(state): State<Arc<ServerState>>,
	Path(identifier): Path<String>,
) -> HandlerResult<Json<bark_json::cli::LightningReceiveInfo>> {
	let wallet = state.require_wallet()?;

	let payment_hash = if let Ok(h) = ark::lightning::PaymentHash::from_str(&identifier) {
		h
	} else if let Ok(i) = Bolt11Invoice::from_str(&identifier) {
		i.into()
	} else if let Ok(p) = ark::lightning::Preimage::from_str(&identifier) {
		p.into()
	} else {
		badarg!("identifier is not a valid payment hash, invoice or preimage");
	};

	let state = wallet.lightning_receive_state(payment_hash).await
		.context("Failed to get lightning receive status")?;
	Ok(axum::Json(bark_json::cli::LightningReceiveInfo::from_state(&state)))
}

#[utoipa::path(
	get,
	path = "/sends/{identifier}",
	summary = "Get send status",
	params(
		("identifier" = String, Path, description = "Payment hash or invoice string to search for"),
	),
	responses(
		(status = 200, description = "Returns the Lightning send status", body = bark_json::cli::LightningSendInfo),
		(status = 400, description = "Bad request", body = error::BadRequestError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Returns the status of a specified outgoing Lightning payment, identified by \
		its payment hash or invoice string. The `state` field tracks the payment lifecycle \
		from `start` through `paid` or `failed`; the preimage is included once the payment \
		succeeded, and `failure_reason` once it failed. A failed payment stays failed: paying \
		the same invoice again through this API is refused. \
		If the wallet does not recognize the payment hash, it will return `unknown`. \
		This is a read on the status in the db, so it does not trigger any `sync` before \
		checking the state.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn get_send_status(
	State(state): State<Arc<ServerState>>,
	Path(identifier): Path<String>,
) -> HandlerResult<Json<bark_json::cli::LightningSendInfo>> {
	let wallet = state.require_wallet()?;

	let payment_hash = if let Ok(h) = ark::lightning::PaymentHash::from_str(&identifier) {
		h
	} else if let Ok(i) = Bolt11Invoice::from_str(&identifier) {
		i.into()
	} else {
		badarg!("identifier is not a valid payment hash or invoice");
	};

	let state = wallet.lightning_send_state(payment_hash).await
		.context("Failed to get lightning send status")?;
	Ok(axum::Json(bark_json::cli::LightningSendInfo::from_state(payment_hash, &state)))
}

#[utoipa::path(
	get,
	path = "/receives",
	summary = "List all pending receive statuses",
	responses(
		(status = 200, description = "Returns all pending receive statuses", body = Vec<bark_json::cli::LightningReceiveInfo>),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Returns the statuses of all pending Lightning receives, ordered from oldest \
		to newest. A receive is pending until its `finished_at` timestamp is set, indicating \
		it has settled or been canceled.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn list_receive_statuses(
	State(state): State<Arc<ServerState>>,
) -> HandlerResult<Json<Vec<bark_json::cli::LightningReceiveInfo>>> {
	let wallet = state.require_wallet()?;

	let mut receives = wallet.pending_lightning_receives().await
		.context("Failed to get lightning receives")?;
	// receives are ordered from newest to oldest, so we reverse them so last terminal item is newest
	receives.reverse();

	let receives = receives.iter()
		.map(bark_json::cli::LightningReceiveInfo::from).collect::<Vec<_>>();

	Ok(axum::Json(receives))
}

#[utoipa::path(
	delete,
	path = "/receives/{identifier}",
	summary = "Cancel a pending receive",
	params(
		("identifier" = String, Path, description = "Payment hash or invoice string"),
	),
	responses(
		(status = 200, description = "Receive canceled successfully"),
		(status = 400, description = "Bad request", body = error::BadRequestError),
		(status = 404, description = "Not found", body = error::NotFoundError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Cancels a pending Lightning receive identified by its payment hash or \
		invoice string. The server will refuse cancellation if HTLC-recv VTXOs have already \
		been granted. Bark also prevents cancellation when the preimage has been revealed.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn cancel_receive(
	State(state): State<Arc<ServerState>>,
	Path(identifier): Path<String>,
) -> HandlerResult<()> {
	let wallet = state.require_wallet()?;

	let payment_hash = if let Ok(h) = ark::lightning::PaymentHash::from_str(&identifier) {
		h
	} else if let Ok(i) = Bolt11Invoice::from_str(&identifier) {
		i.into()
	} else {
		badarg!("identifier is not a valid payment hash or invoice");
	};

	wallet.cancel_lightning_receive(payment_hash).await
		.context("Failed to cancel lightning receive")?;

	Ok(())
}

#[utoipa::path(
	post,
	path = "/pay",
	summary = "Send a Lightning payment",
	request_body = bark_json::web::LightningPayRequest,
	responses(
		(status = 200, description = "Returns success message, optionally with \
			preimage if payment was immediately settled", body = bark_json::web::LightningPayResponse),
		(status = 400, description = "The provided destination is not a valid \
			BOLT11 invoice, BOLT12 offer, or Lightning address", body = error::BadRequestError),
		(status = 500, description = "Internal server error", body = error::InternalServerError)
	),
	description = "Sends a payment to a Lightning destination. Accepts a BOLT11 invoice, \
		BOLT12 offer, or Lightning address. The `amount_sat` field is required for Lightning \
		addresses but optional for invoices and offers. Comments are only supported for \
		Lightning addresses. With `max_fee_sat`, the payment is refused before any funds are \
		committed if its fee would be higher. Paying an invoice whose earlier payment failed is \
		refused, so a failure stays final. Returns once the payment has been started, so its \
		status can be polled right away.",
	tag = "lightning"
)]
#[debug_handler]
pub async fn pay(
	State(state): State<Arc<ServerState>>,
	Json(body): Json<bark_json::web::LightningPayRequest>,
) -> HandlerResult<Json<bark_json::web::LightningPayResponse>> {
	let wallet = state.require_wallet()?;

	let amount = body.amount_sat.map(|a| Amount::from_sat(a));
	let opts = LightningSendOptions::default()
		.retry_for(body.retry_for_secs.map(Duration::from_secs))
		.max_fee(body.max_fee_sat.map(Amount::from_sat))
		// A failed payment reported by `GET /sends/{identifier}` stays failed.
		.refuse_after_failure(true);

	let invoice = if let Ok(invoice) = Bolt11Invoice::from_str(&body.destination) {
		if body.comment.is_some() {
			badarg!("comment is not supported for BOLT-11 invoices");
		}
		wallet.pay_lightning_invoice_with(invoice, amount, opts).await?
	} else if let Ok(offer) = Offer::from_str(&body.destination) {
		if body.comment.is_some() {
			badarg!("comment is not supported for BOLT-12 offers");
		}
		wallet.pay_lightning_offer_with(offer, amount, opts).await?
	} else if let Ok(lnaddr) = LightningAddress::from_str(&body.destination) {
		let amount = amount.badarg("amount is required for Lightning addresses")?;
		wallet.pay_lightning_address_with(&lnaddr, amount, body.comment, opts).await?
	} else if let Ok(lnurl) = LnUrl::from_str(&body.destination) {
		let amount = amount.badarg("amount is required for LNURL")?;
		wallet.pay_lnurl_with(&lnurl, amount, body.comment, opts).await?
	} else {
		badarg!("argument is not a valid BOLT-11 invoice, BOLT-12 offer, Lightning address or LNURL");
	};

	Ok(axum::Json(bark_json::web::LightningPayResponse {
		message: "Payment initiated successfully".to_string(),
		payment_hash: Some(invoice.payment_hash()),
	}))
}

#[cfg(test)]
mod test {
	use super::*;

	/// The router panics on construction if routes conflict, e.g. if the
	/// path parameter of the hold receive routes is named unlike that of
	/// their siblings.
	#[test]
	fn router_routes_do_not_conflict() {
		let _ = router();
	}
}
