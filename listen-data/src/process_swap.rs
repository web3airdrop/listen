use std::sync::Arc;

use crate::constants::WSOL_MINT_KEY_STR;
use crate::diffs::{
    get_token_balance_diff, process_diffs, Diff, DiffsError, DiffsResult,
};
use crate::{
    db::{ClickhouseDb, Database},
    kv_store::RedisKVStore,
    message_queue::{MessageQueue, RedisMessageQueue},
    metadata::get_token_metadata,
    metrics::SwapMetrics,
    price::PriceUpdate,
    sol_price_stream::get_sol_price,
};
use anyhow::{Context, Result};
use carbon_core::transaction::TransactionMetadata;
use chrono::Utc;
use tracing::{debug, warn};

pub async fn process_swap(
    transaction_metadata: &TransactionMetadata,
    message_queue: &RedisMessageQueue,
    kv_store: &Arc<RedisKVStore>,
    db: &Arc<ClickhouseDb>,
    metrics: &SwapMetrics,
) -> Result<()> {
    let diffs = get_token_balance_diff(
        transaction_metadata
            .meta
            .pre_token_balances
            .as_ref()
            .unwrap(),
        transaction_metadata
            .meta
            .post_token_balances
            .as_ref()
            .unwrap(),
    );

    if diffs.iter().all(|d| d.diff.abs() < 0.01) {
        debug!("skipping tiny diffs");
        metrics.increment_skipped_tiny_swaps();
        return Ok(());
    }

    if diffs.iter().any(|d| d.diff == 0.0) {
        debug!("skipping zero diffs (arbitrage likely)");
        metrics.increment_skipped_zero_swaps();
        return Ok(());
    }

    let sol_price = get_sol_price().await;

    if diffs.len() > 3 || diffs.len() < 2 {
        debug!(
            "https://solscan.io/tx/{} skipping swap with unexpected number of tokens: {}",
            transaction_metadata.signature, diffs.len()
        );
        metrics.increment_skipped_unexpected_number_of_tokens();
        return Ok(());
    }

    // Handle multi-hop swaps (3 tokens)
    if diffs.len() == 3 {
        metrics.increment_multi_hop_swap();

        // TODO
        // even though multi-hop swaps are ~1.5% of all swaps, some of those are
        // 4-5 figs, important for volume estimation for latest price those are
        // not crucial since every slot there will be a price update but the
        // price/mc has to be calculated by a more complex formula
        //
        // are all examples of transactions where both raydium and whirlpool or meteora are used simultaneously
        // https://solscan.io/tx/31pB39KowUTdDSjXhzCYi7QxVSWSM4ZijaSWAkCduWUUR6GuGrWwVBbcXLLdJnVLrWbQaV7YFL2SigBXRatGfnji#tokenBalanceChange
        // https://solscan.io/tx/5j8nDKNNLbXcJqdZvM7h76m2tuzjMsbAWAo8vfSPwBMjnVaE47G6uXbG9NE6GFHth76K9qzJMBzNeM2xoHHkT3qZ#tokenBalanceChange
        // https://solscan.io/tx/3m4LERWUekW7im8rgu8QgpSJA8a9yEYL3gDvorbd5YpkXarrL3PGoVmyFyQzd1Pw9oZiQy2LPUjaG8Xr4p433kwn#tokenBalanceChange
        //
        // two options
        // 1) skip all multi-hops
        // 2) detect multi-hops that go out of raydium and meteora, process them separately
        // 3) support multi-hops cross-any-dex
        //
        // for now going with 1)
        //
        // volume is normalized, since skipping multi-hops for every token means
        // some portion of multi-hops are raydium to raydium, token A to token
        // B, thus A->SOL->C, those are supported but since detecting is
        // error-prone and we cannot afford to receive wrong prices by the
        // listen-trading-engine, skipping all multi-hops for now
        //
        // volume is normalized, since skipping multi-hops for every token means
        // each will lose equally as much volume
        //
        // while 2) could work, jupiter has 12+ aggregator accounts which
        // take part in transaction OKX also has an aggregator, it is not
        // trivial so ideally, it could be 3)
        //
        // now for 3) - it is actually possible to change up the `diffs` module
        // to not aggregate by raydium owner + mint, and allow multiple SOL
        // pools accounts then match the pools to the diffs and calculate the
        // pricing for each account separately this is the most appealing
        // approach since raydium accounts will have the correct price, amounts
        // are split but ratios are respected; for now (14th Feb '25) there are
        // other priorities, but this is the way to go
        return Ok(());
        // Find the tokens with positive and negative changes
        #[allow(unreachable_code)]
        let mut positive_diff = None;
        let mut negative_diff = None;
        let mut sol_diff = None;

        for diff in &diffs {
            if diff.mint == WSOL_MINT_KEY_STR {
                sol_diff = Some(diff);
                continue;
            }
            if diff.diff > 0.0 {
                positive_diff = Some(diff);
            } else if diff.diff < 0.0 {
                negative_diff = Some(diff);
            }
        }

        if positive_diff.is_none()
            || negative_diff.is_none()
            || sol_diff.is_none()
        {
            debug!(
                "https://solscan.io/tx/{} three diff swap with unexpected token changes",
                transaction_metadata.signature
            );
            metrics.increment_skipped_unexpected_number_of_tokens();
            return Ok(());
        }

        if let (Some(pos), Some(neg), Some(sol)) =
            (positive_diff, negative_diff, sol_diff)
        {
            // Process first hop: token being sold to SOL
            process_two_token_swap(
                &[neg.clone(), sol.clone()],
                transaction_metadata,
                message_queue,
                kv_store,
                db,
                metrics,
                sol_price,
                true,
            )
            .await
            .context("failed to process first hop")?;

            // Process second hop: SOL to token being bought
            process_two_token_swap(
                &[pos.clone(), sol.clone()],
                transaction_metadata,
                message_queue,
                kv_store,
                db,
                metrics,
                sol_price,
                true,
            )
            .await
            .context("failed to process second hop")?;

            return Ok(());
        }
    }

    process_two_token_swap(
        &diffs,
        transaction_metadata,
        message_queue,
        kv_store,
        db,
        metrics,
        sol_price,
        false,
    )
    .await
    .context("failed to process two token swap")
}

// Helper function to process a single two-token swap
#[allow(clippy::too_many_arguments)]
async fn process_two_token_swap(
    diffs: &[Diff],
    transaction_metadata: &TransactionMetadata,
    message_queue: &RedisMessageQueue,
    kv_store: &Arc<RedisKVStore>,
    db: &Arc<ClickhouseDb>,
    metrics: &SwapMetrics,
    sol_price: f64,
    multi_hop: bool,
) -> Result<()> {
    let DiffsResult {
        price,
        swap_amount,
        coin_mint,
        is_buy,
    } = match process_diffs(diffs, sol_price) {
        Ok(result) => result,
        Err(e) => {
            match e {
                DiffsError::NonWsolsSwap => {
                    metrics.increment_skipped_non_wsol();
                }
                DiffsError::ExpectedExactlyTwoTokenBalanceDiffs => {
                    metrics.increment_skipped_unexpected_number_of_tokens();
                }
            }
            return Ok(());
        }
    };

    // Get metadata and emit price update
    let token_metadata = match get_token_metadata(kv_store, &coin_mint).await {
        Ok(Some(metadata)) => metadata,
        Ok(None) => {
            debug!(
                "https://solscan.io/tx/{} failed to get token metadata",
                transaction_metadata.signature
            );
            metrics.increment_skipped_no_metadata();
            return Ok(());
        }
        Err(e) => {
            warn!(
                "https://solscan.io/tx/{} failed to get token metadata: {}",
                transaction_metadata.signature, e
            );
            metrics.increment_skipped_no_metadata();
            return Ok(());
        }
    };

    // Calculate market cap if we have the metadata
    let market_cap = {
        let supply = token_metadata.spl.supply as f64;
        let adjusted_supply =
            supply / (10_f64.powi(token_metadata.spl.decimals as i32));
        price * adjusted_supply
    };

    let is_pump = token_metadata
        .mpl
        .ipfs_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("createdOn"))
        .is_some_and(|value| {
            value.as_str().is_some_and(|s| s.contains("pump.fun"))
        });

    let price_update = PriceUpdate {
        name: token_metadata.mpl.name,
        pubkey: coin_mint,
        price,
        market_cap,
        timestamp: Utc::now().timestamp() as u64,
        slot: transaction_metadata.slot,
        swap_amount,
        owner: transaction_metadata.fee_payer.to_string(),
        signature: transaction_metadata.signature.to_string(),
        multi_hop,
        is_buy,
        is_pump,
    };

    match db.insert_price(&price_update).await {
        Ok(_) => metrics.increment_db_insert_success(),
        Err(e) => {
            metrics.increment_db_insert_failure();
            return Err(e);
        }
    }

    match message_queue
        .publish_price_update(price_update.clone())
        .await
    {
        Ok(_) => metrics.increment_message_send_success(),
        Err(e) => {
            metrics.increment_message_send_failure();
            return Err(e.into());
        }
    }

    match kv_store.insert_price(&price_update).await {
        Ok(_) => metrics.increment_kv_insert_success(),
        Err(e) => {
            metrics.increment_kv_insert_failure();
            return Err(e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        diffs::Diff,
        util::{make_rpc_client, round_to_decimals},
    };

    use super::*;

    #[tokio::test]
    async fn test_sol_for_token() {
        let diffs = vec![
            Diff {
                mint: "G6ZaVuWEuGtFRooaiHQWjDzoCzr2f7BWr3PhsQRnjSTE"
                    .to_string(),
                pre_amount: 9502698.632123,
                post_amount: 9493791.483438,
                diff: -8907.148685000837,
                owner: "8CNuwDVRshWyZtWRvgb31AMaBge4q6KSRHNPdJHP29HU"
                    .to_string(),
            },
            Diff {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                pre_amount: 145.774357667,
                post_amount: 142.421949398,
                diff: -3.3524082689999943,
                owner: "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1"
                    .to_string(),
            },
        ];

        let DiffsResult {
            price, swap_amount, ..
        } = process_diffs(&diffs, 201.36).unwrap();
        let rounded_price = round_to_decimals(price, 4);
        assert!(rounded_price == 0.0758, "price: {}", rounded_price);
        assert!(
            swap_amount == 3.3524082689999943 * 201.36,
            "swap_amount: {}",
            swap_amount
        );
    }

    #[tokio::test]
    async fn test_sol_for_token_2() {
        let diffs = vec![
            Diff {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                pre_amount: 450.295597127,
                post_amount: 450.345597127,
                diff: 0.05000000000001137,
                owner: "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1"
                    .to_string(),
            },
            Diff {
                mint: "CSChJMDH1drnxaN5ZXr8ZPZtqXv2FJqNTGcSujyfmoon"
                    .to_string(),
                pre_amount: 61602947.9232689,
                post_amount: 61596125.50088912,
                diff: -6822.422379776835,
                owner: "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1"
                    .to_string(),
            },
        ];

        let DiffsResult {
            price, swap_amount, ..
        } = process_diffs(&diffs, 202.12).unwrap();
        let rounded_price = round_to_decimals(price, 5);
        assert!(rounded_price == 0.00148, "price: {}", rounded_price);
        assert!(
            swap_amount == 0.05000000000001137 * 202.12,
            "swap_amount: {}",
            swap_amount
        );
    }

    #[tokio::test]
    async fn test_by_signature() {
        let signature = "538voMuFQKp3oE6Tu598R8kJN12sum2cGMxZBxrV2Vuip1TL4qdWaXiJ8u3yRxgJy9SFX4faP2zC83oDX68D2wuW";
        let transaction = make_rpc_client()
            .unwrap()
            .get_transaction_with_config(
                &signature.parse().unwrap(),
                solana_client::rpc_config::RpcTransactionConfig {
                    encoding: Some(solana_transaction_status::UiTransactionEncoding::JsonParsed),
                    max_supported_transaction_version: Some(0),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let transaction_meta = transaction.transaction.meta.unwrap();

        let diffs = get_token_balance_diff(
            transaction_meta.pre_token_balances.as_ref().unwrap(),
            transaction_meta.post_token_balances.as_ref().unwrap(),
        );
        println!("diffs: {:#?}", diffs);
        let DiffsResult {
            price, swap_amount, ..
        } = process_diffs(&diffs, 203.67).unwrap();
        let rounded_price = round_to_decimals(price, 5);
        assert!(rounded_price == 0.00035, "price: {}", rounded_price);
        let rounded_swap_amount = round_to_decimals(swap_amount, 4);
        assert!(
            rounded_swap_amount == 0.8618,
            "swap_amount: {}",
            rounded_swap_amount
        );
    }

    #[tokio::test]
    async fn test_by_signature_2() {
        let signature = "3m4LERWUekW7im8rgu8QgpSJA8a9yEYL3gDvorbd5YpkXarrL3PGoVmyFyQzd1Pw9oZiQy2LPUjaG8Xr4p433kwn";
        let transaction = make_rpc_client()
            .unwrap()
            .get_transaction_with_config(
                &signature.parse().unwrap(),
                solana_client::rpc_config::RpcTransactionConfig {
                    encoding: Some(solana_transaction_status::UiTransactionEncoding::JsonParsed),
                    max_supported_transaction_version: Some(0),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let transaction_meta = transaction.transaction.meta.unwrap();

        let diffs = get_token_balance_diff(
            transaction_meta.pre_token_balances.as_ref().unwrap(),
            transaction_meta.post_token_balances.as_ref().unwrap(),
        );

        println!("pre: {:#?}", transaction_meta.pre_token_balances);
        println!("post: {:#?}", transaction_meta.post_token_balances);

        println!("diffs: {:#?}", diffs);
    }
}
