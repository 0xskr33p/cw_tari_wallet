use crate::api::{db::get_db_connection, error::WalletError, utils::format_micro_tari};
use anyhow::Result;
use flutter_rust_bridge::frb;
use minotari_wallet::{
    db::{
        get_displayed_transactions_needing_confirmation_update,
        get_displayed_transactions_paginated, get_latest_scanned_tip_block_by_account,
        update_displayed_transaction_confirmations,
    },
    get_accounts,
    utils::timestamp::format_timestamp,
};
use tari_common_types::{payment_reference::generate_payment_reference, types::FixedHash};
/// Fee charged on a transaction: `amount` in **microTari** plus a pre-formatted
/// `amount_display` string for the UI.
#[frb]
#[derive(Debug, Clone)]
pub struct FeeInfoDto {
    pub amount: u64,
    pub amount_display: String,
}

impl From<minotari_wallet::transactions::FeeInfo> for FeeInfoDto {
    fn from(f: minotari_wallet::transactions::FeeInfo) -> Self {
        Self {
            amount: f.amount.0,
            amount_display: format_micro_tari(f.amount.0),
        }
    }
}

/// Where a transaction sits on chain: `block_height`, a formatted `timestamp`
/// string, and the number of `confirmations` (blocks) on top of it.
#[frb]
#[derive(Debug, Clone)]
pub struct BlockchainInfoDto {
    pub block_height: u64,
    pub timestamp: String,
    pub confirmations: u64,
}

impl From<minotari_wallet::transactions::BlockchainInfo> for BlockchainInfoDto {
    fn from(i: minotari_wallet::transactions::BlockchainInfo) -> Self {
        Self {
            block_height: i.block_height,
            timestamp: format_timestamp(i.timestamp),
            confirmations: i.confirmations,
        }
    }
}

/// The other party to a transaction: their base58 `address` and the same
/// address rendered as an `address_emoji` string.
#[frb]
#[derive(Debug, Clone)]
pub struct CounterpartyInfoDto {
    pub address: String,
    pub address_emoji: String,
}

impl From<tari_common_types::tari_address::TariAddress> for CounterpartyInfoDto {
    fn from(addr: tari_common_types::tari_address::TariAddress) -> Self {
        Self {
            address: addr.to_base58(),
            address_emoji: addr.to_emoji_string(),
        }
    }
}

/// Whether a transaction credited (`Incoming`) or debited (`Outgoing`) the wallet.
#[frb]
#[derive(Debug, Clone)]
pub enum DisplayedTransactionDirection {
    Incoming,
    Outgoing,
}

impl From<minotari_wallet::transactions::TransactionDirection> for DisplayedTransactionDirection {
    fn from(d: minotari_wallet::transactions::TransactionDirection) -> Self {
        match d {
            minotari_wallet::transactions::TransactionDirection::Incoming => Self::Incoming,
            minotari_wallet::transactions::TransactionDirection::Outgoing => Self::Outgoing,
        }
    }
}

/// How a transaction originated: a regular `Transfer`, mining `Coinbase`,
/// `OneSided` payment, or `Unknown`.
#[frb]
#[derive(Debug, Clone)]
pub enum DisplayedTransactionSource {
    Transfer,
    Coinbase,
    OneSided,
    Unknown,
}

impl From<minotari_wallet::transactions::TransactionSource> for DisplayedTransactionSource {
    fn from(s: minotari_wallet::transactions::TransactionSource) -> Self {
        match s {
            minotari_wallet::transactions::TransactionSource::Transfer => Self::Transfer,
            minotari_wallet::transactions::TransactionSource::Coinbase => Self::Coinbase,
            minotari_wallet::transactions::TransactionSource::OneSided => Self::OneSided,
            minotari_wallet::transactions::TransactionSource::Unknown => Self::Unknown,
        }
    }
}

/// The display status of a transaction as shown in the wallet UI.
#[frb]
#[derive(Debug, Clone)]
pub enum DisplayedTransactionStatus {
    Pending,
    Unconfirmed,
    Confirmed,
    Cancelled,
    Reorganized,
    Rejected,
    Locked,
}

impl From<minotari_wallet::transactions::TransactionDisplayStatus> for DisplayedTransactionStatus {
    fn from(s: minotari_wallet::transactions::TransactionDisplayStatus) -> Self {
        match s {
            minotari_wallet::transactions::TransactionDisplayStatus::Pending => Self::Pending,
            minotari_wallet::transactions::TransactionDisplayStatus::Unconfirmed => {
                Self::Unconfirmed
            }
            minotari_wallet::transactions::TransactionDisplayStatus::Confirmed => Self::Confirmed,
            minotari_wallet::transactions::TransactionDisplayStatus::Cancelled => Self::Cancelled,
            minotari_wallet::transactions::TransactionDisplayStatus::Reorganized => {
                Self::Reorganized
            }
            minotari_wallet::transactions::TransactionDisplayStatus::Rejected => Self::Rejected,
            minotari_wallet::transactions::TransactionDisplayStatus::Locked => Self::Locked,
        }
    }
}

/// A transaction as displayed in the wallet UI.
///
/// `amount` is in **microTari** with a pre-formatted `amount_display`. `counterparty`
/// and `fee` are absent for some kinds. `payrefs` are hex payment-reference hashes.
#[frb]
#[derive(Debug, Clone)]
pub struct DisplayedTransactionDto {
    pub id: String,
    pub direction: DisplayedTransactionDirection,
    pub source: DisplayedTransactionSource,
    pub status: DisplayedTransactionStatus,
    pub amount: u64,
    pub amount_display: String,
    pub message: Option<String>,
    pub counterparty: Option<CounterpartyInfoDto>,
    pub blockchain: BlockchainInfoDto,
    pub fee: Option<FeeInfoDto>,
    pub payrefs: Vec<String>,
}

impl From<minotari_wallet::DisplayedTransaction> for DisplayedTransactionDto {
    fn from(t: minotari_wallet::DisplayedTransaction) -> Self {
        // Incoming receipts need references for our received outputs, even when
        // the sender's optional memo contains no sent_output_hashes.
        let mut payrefs = Vec::new();
        let is_mined = t.blockchain.block_height > 0
            && t.blockchain.block_hash != FixedHash::zero()
            && !matches!(
                t.status,
                minotari_wallet::transactions::TransactionDisplayStatus::Reorganized
                    | minotari_wallet::transactions::TransactionDisplayStatus::Cancelled
                    | minotari_wallet::transactions::TransactionDisplayStatus::Rejected
            );
        if is_mined {
            if matches!(
                t.direction,
                minotari_wallet::transactions::TransactionDirection::Incoming
            ) {
                for output in &t.details.outputs {
                    if !output.is_change
                        && output.mined_in_block_height == t.blockchain.block_height
                        && output.mined_in_block_hash == t.blockchain.block_hash
                        && output.hash != FixedHash::zero()
                    {
                        let reference =
                            generate_payment_reference(&output.mined_in_block_hash, &output.hash)
                                .to_string();
                        if !payrefs.contains(&reference) {
                            payrefs.push(reference);
                        }
                    }
                }
            }
            // Retain the existing send/coinbase path when no received output
            // can be reconstructed from this history entry.
            if payrefs.is_empty() {
                payrefs = t
                    .details
                    .sent_payrefs
                    .iter()
                    .filter(|reference| **reference != FixedHash::zero())
                    .map(ToString::to_string)
                    .collect();
            }
        }
        Self {
            id: t.id.to_string(),
            direction: t.direction.into(),
            source: t.source.into(),
            status: t.status.into(),
            amount: t.amount.0,
            amount_display: format_micro_tari(t.amount.0),
            message: t.message,
            counterparty: t.counterparty.map(CounterpartyInfoDto::from),
            blockchain: t.blockchain.into(),
            fee: t.fee.map(FeeInfoDto::from),
            payrefs,
        }
    }
}

/// List a page of the wallet's transactions, most recent first.
///
/// `limit`/`offset` paginate the result. Requires
/// [`initialize_database`](crate::api::db::initialize_database) first.
/// Synchronous; errors if the account does not exist.
#[frb]
pub fn get_transactions(
    wallet_name: String,
    limit: i64,
    offset: i64,
) -> Result<Vec<DisplayedTransactionDto>> {
    let mut conn = get_db_connection()?;
    let accounts = &get_accounts(&mut conn, Some(&wallet_name))?;
    let account = accounts.first().ok_or(WalletError::NoAccounts)?;

    let transactions = get_displayed_transactions_paginated(&conn, account.id, limit, offset)?;

    Ok(transactions.into_iter().map(Into::into).collect())
}

/// Refresh mined Pending/Unconfirmed/Locked entries using this account's scanned tip.
///
/// Workaround for minotari 0.3.0's skipped transaction monitor, matching Universe.
/// Call between scan cycles. Unmined sends and removed transactions are preserved.
/// Returns changed entries; existing scan events and read APIs are unchanged.
#[frb]
pub fn refresh_transaction_confirmations(
    wallet_name: String,
    required_confirmations: Option<u64>,
) -> Result<Vec<DisplayedTransactionDto>> {
    let required =
        required_confirmations.unwrap_or(crate::api::config::DEFAULT_CONFIRMATION_WINDOW);
    anyhow::ensure!(
        required > 0,
        "Required confirmations must be greater than zero"
    );
    let mut conn = get_db_connection()?;
    let account = get_accounts(&mut conn, Some(&wallet_name))?
        .into_iter()
        .next()
        .ok_or(WalletError::NoAccounts)?;
    let snapshot = conn.transaction()?;
    let Some(tip) = get_latest_scanned_tip_block_by_account(&snapshot, account.id)? else {
        return Ok(Vec::new());
    };
    let mut updated = Vec::new();
    for mut tx in get_displayed_transactions_needing_confirmation_update(&snapshot, account.id)? {
        if refresh_mined_status(&mut tx, tip.height, required) {
            update_displayed_transaction_confirmations(&snapshot, &tx)?;
            updated.push(tx.into());
        }
    }
    snapshot.commit()?;
    Ok(updated)
}

fn refresh_mined_status(
    tx: &mut minotari_wallet::DisplayedTransaction,
    scanned_height: u64,
    required: u64,
) -> bool {
    use minotari_wallet::transactions::TransactionDisplayStatus as Status;
    if !matches!(
        tx.status,
        Status::Pending | Status::Unconfirmed | Status::Locked
    ) || tx.blockchain.block_height == 0
        || tx.blockchain.block_height > scanned_height
        || tx.blockchain.block_hash == FixedHash::zero()
    {
        return false;
    }
    let confirmations = scanned_height.saturating_sub(tx.blockchain.block_height);
    let status = if confirmations >= required {
        if tx.lock_height > scanned_height {
            Status::Locked
        } else {
            Status::Confirmed
        }
    } else if confirmations > 0 {
        Status::Unconfirmed
    } else {
        Status::Pending
    };
    if confirmations == tx.blockchain.confirmations && status == tx.status {
        return false;
    }
    tx.blockchain.confirmations = confirmations;
    tx.status = status;
    true
}

#[cfg(test)]
mod tests {
    //! Enum-mapping tests. Pure conversions, no I/O. These are upstream-drift
    //! tripwires: if `minotari`'s transaction enums add/rename a variant, the
    //! `From` impls (and these exhaustive assertions) must be revisited.
    //!
    //! The DTO enums derive only `Debug`/`Clone` (no `PartialEq`), so equality is
    //! asserted with `matches!` against the expected variant.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use minotari_wallet::models::OutputStatus;
    use minotari_wallet::transactions::{
        BlockchainInfo, TransactionDetails, TransactionDirection, TransactionDisplayStatus,
        TransactionOutput, TransactionSource,
    };
    use tari_transaction_components::{
        tari_amount::MicroMinotari, transaction_components::OutputType,
    };
    use tari_utilities::hex::Hex;

    const RECEIVED_PAYREF: &str =
        "9cb2bb582664d051387c2d40ed638be2d7d98226b64ced71f90827a8e73f5a1a";

    #[test]
    fn mined_receipt_crosses_confirmation_threshold_and_keeps_its_reference() {
        let mut tx = received_transaction();
        assert!(refresh_mined_status(&mut tx, 363576, 3));
        assert_eq!(tx.blockchain.confirmations, 2);
        assert_eq!(tx.status, TransactionDisplayStatus::Unconfirmed);
        assert!(refresh_mined_status(&mut tx, 363577, 3));
        assert_eq!(tx.status, TransactionDisplayStatus::Confirmed);
        assert_eq!(
            DisplayedTransactionDto::from(tx).payrefs,
            vec![RECEIVED_PAYREF]
        );
    }

    #[test]
    fn mined_lock_stays_locked_until_its_height_and_can_change_status_only() {
        let mut tx = received_transaction();
        tx.lock_height = 363580;
        assert!(refresh_mined_status(&mut tx, 363577, 3));
        assert_eq!(tx.status, TransactionDisplayStatus::Locked);
        assert!(!refresh_mined_status(&mut tx, 363577, 3));
        assert!(refresh_mined_status(&mut tx, 363580, 3));
        assert_eq!(tx.status, TransactionDisplayStatus::Confirmed);
        tx.status = TransactionDisplayStatus::Unconfirmed;
        assert!(refresh_mined_status(&mut tx, 363580, 3));
        assert_eq!(tx.status, TransactionDisplayStatus::Confirmed);
    }

    #[test]
    fn unmined_unknown_or_removed_entries_are_not_promoted() {
        for status in [
            TransactionDisplayStatus::Cancelled,
            TransactionDisplayStatus::Rejected,
            TransactionDisplayStatus::Reorganized,
            TransactionDisplayStatus::Confirmed,
        ] {
            let mut tx = received_transaction();
            tx.status = status;
            assert!(!refresh_mined_status(&mut tx, 363714, 3));
            assert_eq!(tx.status, status);
            assert_eq!(tx.blockchain.confirmations, 0);
        }
        let mut tx = received_transaction();
        tx.direction = TransactionDirection::Outgoing;
        tx.blockchain.block_height = 0;
        tx.status = TransactionDisplayStatus::Pending;
        assert!(!refresh_mined_status(&mut tx, 363714, 3));
        assert_eq!(tx.status, TransactionDisplayStatus::Pending);
        tx.blockchain.block_height = 363715;
        assert!(!refresh_mined_status(&mut tx, 363714, 3));
        tx.blockchain.block_height = 363574;
        tx.blockchain.block_hash = FixedHash::zero();
        assert!(!refresh_mined_status(&mut tx, 363714, 3));
    }

    fn received_transaction() -> minotari_wallet::DisplayedTransaction {
        // Public output/block hashes from the funded integration wallet, also
        // checked against the explorer. No wallet keys or seed are required.
        let block_hash =
            FixedHash::from_hex("35222fd500aae8c471b37885dcded36767afde6ea7609daaedbbe9e01a47b442")
                .unwrap();
        let output_hash =
            FixedHash::from_hex("d672aebab31aa1cf521ddc1f85ba93587e6393acc1fca0a810f01c38211c3683")
                .unwrap();
        minotari_wallet::DisplayedTransaction {
            id: 42_u64.into(),
            direction: TransactionDirection::Incoming,
            source: TransactionSource::Transfer,
            status: TransactionDisplayStatus::Unconfirmed,
            amount: MicroMinotari(100_000_000),
            message: None,
            counterparty: None,
            blockchain: BlockchainInfo {
                block_height: 363574,
                timestamp: chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
                confirmations: 0,
                block_hash,
            },
            fee: None,
            details: TransactionDetails {
                account_id: 1,
                total_credit: MicroMinotari(100_000_000),
                total_debit: MicroMinotari(0),
                inputs: vec![],
                outputs: vec![TransactionOutput {
                    hash: output_hash,
                    amount: MicroMinotari(100_000_000),
                    status: OutputStatus::Unspent,
                    mined_in_block_height: 363574,
                    mined_in_block_hash: block_hash,
                    output_type: OutputType::Standard,
                    is_change: false,
                }],
                output_type: None,
                coinbase_extra: None,
                memo_hex: None,
                sent_output_hashes: vec![],
                sent_payrefs: vec![],
            },
            lock_height: 0,
        }
    }

    #[test]
    fn incoming_without_sent_payrefs_exports_the_explorer_reference() {
        let dto = DisplayedTransactionDto::from(received_transaction());
        assert_eq!(dto.payrefs, vec![RECEIVED_PAYREF]);
        assert_eq!(dto.id, "42");
        assert!(matches!(
            dto.status,
            DisplayedTransactionStatus::Unconfirmed
        ));
    }

    #[test]
    fn incoming_exports_all_received_outputs_without_change_or_duplicates() {
        let mut tx = received_transaction();
        let first = tx.details.outputs[0].clone();
        let mut second = first.clone();
        second.hash = FixedHash::from([2_u8; 32]);
        let second_reference =
            generate_payment_reference(&second.mined_in_block_hash, &second.hash).to_string();
        let mut change = first.clone();
        change.hash = FixedHash::from([3_u8; 32]);
        change.is_change = true;
        tx.details.outputs.extend([first, second, change]);
        tx.details.sent_payrefs.push(FixedHash::from([7_u8; 32]));
        assert_eq!(
            DisplayedTransactionDto::from(tx).payrefs,
            vec![RECEIVED_PAYREF.to_string(), second_reference]
        );
    }

    #[test]
    fn unmined_or_removed_transactions_have_no_current_payrefs() {
        let mut unmined = received_transaction();
        unmined.blockchain.block_height = 0;
        assert!(DisplayedTransactionDto::from(unmined).payrefs.is_empty());

        let mut unknown_block = received_transaction();
        unknown_block.blockchain.block_hash = FixedHash::zero();
        assert!(DisplayedTransactionDto::from(unknown_block)
            .payrefs
            .is_empty());

        for status in [
            TransactionDisplayStatus::Reorganized,
            TransactionDisplayStatus::Cancelled,
            TransactionDisplayStatus::Rejected,
        ] {
            let mut tx = received_transaction();
            tx.status = status;
            tx.details.sent_payrefs.push(FixedHash::from([8_u8; 32]));
            assert!(DisplayedTransactionDto::from(tx).payrefs.is_empty());
        }

        let mut stale_output = received_transaction();
        stale_output.details.outputs[0].mined_in_block_hash = FixedHash::from([9_u8; 32]);
        assert!(DisplayedTransactionDto::from(stale_output)
            .payrefs
            .is_empty());
    }

    #[test]
    fn outgoing_preserves_sent_references_instead_of_our_received_outputs() {
        let mut tx = received_transaction();
        tx.direction = TransactionDirection::Outgoing;
        let reference = FixedHash::from([8_u8; 32]);
        tx.details.sent_payrefs = vec![reference];
        assert_eq!(
            DisplayedTransactionDto::from(tx).payrefs,
            vec![reference.to_string()]
        );
    }

    #[test]
    fn direction_maps_exhaustively() {
        assert!(matches!(
            DisplayedTransactionDirection::from(TransactionDirection::Incoming),
            DisplayedTransactionDirection::Incoming
        ));
        assert!(matches!(
            DisplayedTransactionDirection::from(TransactionDirection::Outgoing),
            DisplayedTransactionDirection::Outgoing
        ));
    }

    #[test]
    fn source_maps_exhaustively() {
        assert!(matches!(
            DisplayedTransactionSource::from(TransactionSource::Transfer),
            DisplayedTransactionSource::Transfer
        ));
        assert!(matches!(
            DisplayedTransactionSource::from(TransactionSource::Coinbase),
            DisplayedTransactionSource::Coinbase
        ));
        assert!(matches!(
            DisplayedTransactionSource::from(TransactionSource::OneSided),
            DisplayedTransactionSource::OneSided
        ));
        assert!(matches!(
            DisplayedTransactionSource::from(TransactionSource::Unknown),
            DisplayedTransactionSource::Unknown
        ));
    }

    #[test]
    fn status_maps_exhaustively() {
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Pending),
            DisplayedTransactionStatus::Pending
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Unconfirmed),
            DisplayedTransactionStatus::Unconfirmed
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Confirmed),
            DisplayedTransactionStatus::Confirmed
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Cancelled),
            DisplayedTransactionStatus::Cancelled
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Reorganized),
            DisplayedTransactionStatus::Reorganized
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Rejected),
            DisplayedTransactionStatus::Rejected
        ));
        assert!(matches!(
            DisplayedTransactionStatus::from(TransactionDisplayStatus::Locked),
            DisplayedTransactionStatus::Locked
        ));
    }
}
