//! This module wraps all of the Solana functionality into rig-compatible tools
//! using the `#[tool]` macro. This allows the functions to be consumed by LLMs
//! as function calls
#![allow(non_upper_case_globals)]

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use reqwest::Client;
use rig_tool_macro::tool;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::native_token::sol_to_lamports;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::common::wrap_unsafe;
use crate::solana::data::PortfolioItem;

use super::data::holdings_to_portfolio;
use super::deploy_token::create_deploy_token_tx;
use super::trade::create_jupiter_swap_transaction;
use super::trade_pump::{create_buy_pump_fun_tx, create_sell_pump_fun_tx};
use super::transfer::{create_transfer_sol_tx, create_transfer_spl_tx};
use super::util::execute_solana_transaction;
use crate::signer::SignerContext;

static SOLANA_RPC_URL: Lazy<String> = Lazy::new(|| {
    std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string())
});

fn create_rpc() -> RpcClient {
    RpcClient::new(SOLANA_RPC_URL.to_string())
}

#[tool(description = "
Fetches a quote from Jupiter API.

Params:
input_mint: string
  public key of the token to swap from
input_amount: string
  amount of the input_mint to swap accounting for decimals, 
  e.g. 1000000 6 decimals, or 1000000000000000000 9 decimals
output_mint: string
  public key of the token to swap to

Might throw for pump.fun tokens that have not migrated to Raydium. This is
different from the Swap tool, works for any Solana token
")]
pub async fn get_quote(
    input_mint: String,
    input_amount: u64,
    output_mint: String,
) -> Result<String> {
    let quote = crate::solana::jup::Jupiter::fetch_quote(
        &input_mint,
        &output_mint,
        input_amount,
    )
    .await
    .map_err(|e| anyhow!("{:#?}", e))?;

    Ok(serde_json::to_string(&quote)?)
}

#[tool(description = "
Performs a swap, choosing the best method. 

Params:
input_mint: string
  public key of the token to swap from
amount: string 
  amount of the input_mint to swap accounting for decimals, 
  e.g. 1000000 6 decimals, or 1000000000000000000 9 decimals
output_mint: string
  public key of the token to swap to

Works for any Solana token, regardless of whether it's on PumpFun, Raydium,
Meteora etc. Will try Jupiter first, and if that fails, will attempt to use 
Pump.fun directly for applicable tokens.

Return:
transaction signature as a string
")]
pub async fn swap(
    input_mint: String,
    amount: String,
    output_mint: String,
) -> Result<String> {
    let _input_mint = input_mint.clone();
    let _amount = amount.clone();
    let _output_mint = output_mint.clone();

    let jupiter_result =
        execute_solana_transaction(move |owner| async move {
            create_jupiter_swap_transaction(
                input_mint.clone(),
                amount.parse::<u64>()?,
                output_mint.clone(),
                &owner,
            )
            .await
            // there would be a slippage error here
        })
        .await;

    // If Jupiter swap succeeds, return the result
    match jupiter_result {
        Ok(signature) => return Ok(signature),
        Err(e) => {
            let jupiter_error = e.to_string();
            if e.to_string().contains("0x1771") {
                return Err(e);
            }
            // Parse the amount from lamports to SOL
            let amount_u64 = _amount.parse::<u64>()?;
            let sol_amount = amount_u64 as f64 / 1_000_000_000.0; // Convert lamports to SOL

            // Try to buy using Pump.fun with a default slippage of 100 bps (1%)
            let pump_res =
                execute_solana_transaction(move |owner| async move {
                    if _input_mint.to_lowercase()
                        == "so11111111111111111111111111111111111111112"
                    {
                        create_buy_pump_fun_tx(
                            _output_mint,
                            sol_to_lamports(sol_amount),
                            100, // 1% slippage
                            &create_rpc(),
                            &owner,
                        )
                        .await
                    } else {
                        create_sell_pump_fun_tx(
                            _input_mint,
                            amount_u64,
                            &owner,
                        )
                        .await
                    }
                })
                .await;

            match pump_res {
                Ok(signature) => return Ok(signature),
                Err(e) => {
                    return Err(anyhow!(
                        "jupiter error: {}\n pump.fun error: {}",
                        jupiter_error,
                        e.to_string()
                    ));
                }
            }
        }
    }
}

#[tool(description = "
Transfers SOL from the current signer to the given address

This function is dangerous, as it can lead to loss of funds if the address is incorrect

ALWAYS double check the to address with the user before calling this function

amount is denoted in lamports, 1 SOL = 10^9 lamports
")]
pub async fn transfer_sol(to: String, amount: u64) -> Result<String> {
    execute_solana_transaction(move |owner| async move {
        create_transfer_sol_tx(&Pubkey::from_str(&to)?, amount, &owner).await
    })
    .await
}

/// param amount is token amount, accounting for decimals
/// e.g. 1 Fartcoin = 1 * 10^6 (6 decimals)
#[tool(description = "
Transfers SPL token from the current signer to the given address

This function is dangerous, as it can lead to loss of funds if the address is incorrect

ALWAYS double check the to address with the user before calling this function

amount is denoted in the token amount, accounting for decimals, if you are unsure
about the decimals, use get_spl_token_balance to get the amount and decimals
")]
pub async fn transfer_spl_token(
    to: String,
    amount: u64,
    mint: String,
) -> Result<String> {
    execute_solana_transaction(move |owner| async move {
        create_transfer_spl_tx(
            &Pubkey::from_str(&to)?,
            amount,
            &Pubkey::from_str(&mint)?,
            &owner,
            &create_rpc(),
        )
        .await
    })
    .await
}

#[tool]
pub async fn get_public_key() -> Result<String> {
    Ok(SignerContext::current().await.pubkey())
}

#[tool]
pub async fn get_sol_balance() -> Result<u64> {
    let signer = SignerContext::current().await.clone();
    let owner = Pubkey::from_str(&signer.pubkey())?;

    wrap_unsafe(move || async move {
        create_rpc()
            .get_balance(&owner)
            .await
            .map_err(|e| anyhow!("{:#?}", e))
    })
    .await
}

#[tool(description = "
get_token_balance returns the amount as String and the decimals as u8
in order to convert to UI amount: amount / 10^decimals
")]
pub async fn get_spl_token_balance(mint: String) -> Result<(String, u8)> {
    let signer = SignerContext::current().await;
    let owner = Pubkey::from_str(&signer.pubkey())?;
    let mint = Pubkey::from_str(&mint)?;
    let ata = spl_associated_token_account::get_associated_token_address(
        &owner, &mint,
    );
    let balance = wrap_unsafe(move || async move {
        create_rpc()
            .get_token_account_balance(&ata)
            .await
            .map_err(|e| anyhow!("{:#?}", e))
    })
    .await
    .map_err(|e| anyhow!("{:#?}", e))?;

    Ok((balance.amount, balance.decimals))
}

#[tool(description = "
PumpFun is a launchpad where anyone can launch a token for around ~$2-3

All of the parameters are required, but for twitter, website, telegram, image_url,
if the user doesnt provide those they can be left as empty strings

The image_url cannot be a local path, it has to be an image url from the
internet, ask user to paste in

dev_buy is denoted in lamports - 1 solana is 10^9 lamports
")]
#[allow(clippy::too_many_arguments)]
pub async fn deploy_pump_fun_token(
    name: String,
    symbol: String,
    twitter: String,
    website: String,
    dev_buy: u64,
    telegram: String,
    image_url: String,
    description: String,
) -> Result<String> {
    execute_solana_transaction(move |owner| async move {
        create_deploy_token_tx(
            crate::solana::deploy_token::DeployTokenParams {
                name,
                symbol,
                twitter: Some(twitter),
                website: Some(website),
                dev_buy: Some(dev_buy),
                telegram: Some(telegram),
                image_url: Some(image_url),
                description,
            },
            &owner,
        )
        .await
    })
    .await
}

#[tool(description = "
Fetches the price of a token from the Jup.ag API that provides latest prices for Solana tokens
")]
pub async fn fetch_token_price(mint: String) -> Result<f64> {
    crate::solana::price::fetch_token_price(mint, &Client::new()).await
}

#[tool(description = "
use this function to buy PumpFun token with SOL

Not every token ending with address Asdf...pump is on pump.fun - first you should try to use the
regular swap function and if it fails with the token being not found, only then try to purchase it
directly on pump

Also, if the user specifically requests to buy on pump.fun, use this method
")]
pub async fn buy_pump_fun_token(
    mint: String,
    sol_amount: f64,
    slippage_bps: u16,
) -> Result<String> {
    execute_solana_transaction(move |owner| async move {
        create_buy_pump_fun_tx(
            mint,
            sol_to_lamports(sol_amount),
            slippage_bps,
            &create_rpc(),
            &owner,
        )
        .await
    })
    .await
}

#[tool(description = "
use this function to sell PumpFun token for SOL

Not every token ending with address Asdf...pump is on pump.fun - first you should try to use the
regular swap function and if it fails with the token being not found, only then try to sell it
directly on pump

Also, if the user specifically requests to sell on pump.fun, use this method
")]
pub async fn sell_pump_fun_token(
    mint: String,
    token_amount: u64,
) -> Result<String> {
    execute_solana_transaction(move |owner| async move {
        create_sell_pump_fun_tx(mint, token_amount, &owner).await
    })
    .await
}

#[tool(description = "
Returns the portfolio of the user, including the amounts, addresses, prices etc

Mostly, the portfolio context will be passed in but this function can be called 
to pull the portfolio again it goes out of the chat context
")]
pub async fn get_portfolio() -> Result<Vec<PortfolioItem>> {
    let owner = Pubkey::from_str(&SignerContext::current().await.pubkey())?;
    let holdings = wrap_unsafe(move || async move {
        crate::solana::balance::get_holdings(&create_rpc(), &owner)
            .await
            .map_err(|e| anyhow!("{:#?}", e))
    })
    .await
    .map_err(|e| anyhow!("{:#?}", e))?;

    holdings_to_portfolio(holdings).await
}
