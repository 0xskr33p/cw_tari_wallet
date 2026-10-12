//! Adapt locally sent history to recipient amount plus the signed kernel fee.

use anyhow::{ensure, Result};
use minotari_wallet::transactions::{DisplayedTransaction, FeeInfo, TransactionDirection};
use tari_transaction_components::transaction_components::Transaction;
use tari_transaction_components::MicroMinotari;

pub(crate) fn kernel_fee(transaction: &Transaction) -> Result<u64> {
    ensure!(
        !transaction.body().kernels().is_empty(),
        "Signed transaction has no kernels"
    );
    transaction
        .body()
        .kernels()
        .iter()
        .try_fold(0_u64, |sum, kernel| {
            sum.checked_add(kernel.fee.0)
                .ok_or_else(|| anyhow::anyhow!("Signed transaction fee overflow"))
        })
}

pub(crate) fn normalize_local_send(tx: &mut DisplayedTransaction, fee: u64) {
    if tx.direction != TransactionDirection::Outgoing {
        return;
    }
    // Upstream's builder stores debit - change, including the fee. Keep its
    // ledger representation in SQLite; subtract exactly once in the UI copy.
    if let Some(net_debit) = tx
        .details
        .total_debit
        .0
        .checked_sub(tx.details.total_credit.0)
    {
        if tx.amount.0 == net_debit {
            if let Some(recipient_amount) = net_debit.checked_sub(fee) {
                tx.amount = MicroMinotari(recipient_amount);
            }
        }
    }
    tx.fee = Some(FeeInfo {
        amount: MicroMinotari(fee),
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use minotari_wallet::transactions::displayed_transaction_processor::DisplayedTransactionBuilder;
    use tari_transaction_components::transaction_components::TransactionKernel;

    fn signed_transaction(fees: &[u64]) -> Transaction {
        Transaction::new(
            vec![],
            vec![],
            fees.iter()
                .map(|fee| {
                    TransactionKernel::new_current_version(
                        Default::default(),
                        MicroMinotari(*fee),
                        0,
                        Default::default(),
                        Default::default(),
                        None,
                    )
                })
                .collect(),
            Default::default(),
            Default::default(),
        )
    }

    #[test]
    fn signed_kernel_fee_is_summed_and_invalid_totals_are_rejected() {
        assert_eq!(kernel_fee(&signed_transaction(&[700])).unwrap(), 700);
        assert_eq!(kernel_fee(&signed_transaction(&[300, 400])).unwrap(), 700);
        assert!(kernel_fee(&signed_transaction(&[])).is_err());
        assert!(kernel_fee(&signed_transaction(&[u64::MAX, 1])).is_err());
    }

    #[test]
    fn send_separates_recipient_amount_and_actual_fee_without_double_subtraction() {
        let mut tx = DisplayedTransactionBuilder::new()
            .account_id(1)
            .credits_and_debits(MicroMinotari(89_999_300), MicroMinotari(100_000_000))
            .fee(Some(MicroMinotari(0)))
            .build(42_u64.into())
            .unwrap();
        normalize_local_send(&mut tx, kernel_fee(&signed_transaction(&[700])).unwrap());
        assert_eq!(tx.amount.0, 10_000_000);
        assert_eq!(tx.fee.as_ref().unwrap().amount.0, 700);
        assert_eq!(tx.details.total_debit.0, 100_000_000);
        assert_eq!(tx.details.total_credit.0, 89_999_300);
        normalize_local_send(&mut tx, 700);
        assert_eq!(tx.amount.0, 10_000_000);
    }

    #[test]
    fn incoming_amount_and_fee_are_preserved() {
        let mut tx = DisplayedTransactionBuilder::new()
            .account_id(1)
            .credits_and_debits(MicroMinotari(100_000_000), MicroMinotari(0))
            .build(42_u64.into())
            .unwrap();
        normalize_local_send(&mut tx, 700);
        assert_eq!(tx.amount.0, 100_000_000);
        assert!(tx.fee.is_none());
    }
}
