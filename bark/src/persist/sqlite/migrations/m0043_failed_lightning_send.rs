//! Keep a permanent record of failed lightning sends.
//!
//! A failed send used to leave no trace once its checkpoint was removed, so
//! its payment hash looked like one the wallet had never seen. Callers that
//! need a failure to stay final (e.g. a lightning gateway) can now look it up,
//! and the wallet refuses to pay the same invoice again.

use anyhow::Context;
use rusqlite::Transaction;

use super::Migration;

pub struct Migration0043 {}

impl Migration for Migration0043 {
	fn name(&self) -> &str {
		"Add failed lightning send fact table"
	}

	fn to_version(&self) -> i64 { 43 }

	fn do_migration(&self, conn: &Transaction) -> anyhow::Result<()> {
		conn.execute(
			"CREATE TABLE bark_failed_lightning_send (
				payment_hash TEXT PRIMARY KEY,
				reason       TEXT NOT NULL,
				failed_at    DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now'))
			)",
			(),
		).context("failed to create bark_failed_lightning_send table")?;

		Ok(())
	}
}
