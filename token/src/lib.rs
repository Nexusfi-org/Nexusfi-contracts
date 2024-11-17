use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::{
    env, 
    near_bindgen, 
    AccountId, 
    PanicOnDefault, 
    Promise, 
    PromiseOrValue, 
    NearToken, 
    Gas,
    PromiseError,
};
use std::collections::HashMap;
use once_cell::sync::Lazy;

pub static TOKEN_ADDRESSES: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("0xe09D8aDae1141181f4CddddeF97E4Cf68f5436E6", "aurora.fakes.testnet");
    m.insert("0x2e5221B0f855Be4ea5Cefffb8311EED0563B6e87", "weth.fakes.testnet");
    m.insert("0xf08a50178dfcde18524640ea6618a1f965821715", "usdc.fakes.testnet");
    m
});

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct NetworkDetails {
    pub chain_id: u64,
    pub eth_nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
}


#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: String,
    pub weight: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PriceData {
    pub multiplier: String,
    pub decimals: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetPrice {
    pub asset_id: String,
    pub price: Option<PriceData>,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OraclePriceData {
    pub timestamp: String,
    pub recency_duration_sec: u64,
    pub prices: Vec<AssetPrice>,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub total_assets: U128,
    pub assets: Vec<AssetInfo>,
    pub owner_id: AccountId,
    pub user_balances: HashMap<AccountId, HashMap<String, U128>>,
    pub usdc_contract: AccountId,
    pub oracle_contract: AccountId,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId, assets: Vec<AssetInfo>, usdc_contract: AccountId) -> Self {
        assert!(!env::state_exists(), "Contract is already initialized");
        let total_weight: u8 = assets.iter().map(|a| a.weight).sum();
        assert_eq!(total_weight, 100, "Total weight of assets must equal 100%");

        Self {
            total_assets: U128(0),
            assets,
            owner_id,
            user_balances: HashMap::new(),
            usdc_contract,
            oracle_contract: "priceoracle.testnet".parse().unwrap(),
        }
    }

    pub fn get_assets(&self) -> Vec<AssetInfo> {
        self.assets.clone()
    }

    pub fn get_total_assets(&self) -> U128 {
        self.total_assets
    }

    pub fn get_number_of_assets(&self) -> usize {
        self.assets.len()
    }

    pub fn get_user_balance(&self, account_id: &AccountId) -> Option<&HashMap<String, U128>> {
        self.user_balances.get(account_id)
    }

    pub fn get_asset_prices(&self) -> Promise {
        Promise::new(self.oracle_contract.clone())
            .function_call(
                "get_price_data".to_string(),
                Vec::new(),
                NearToken::from_near(0),
                Gas::from_tgas(100)
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(50))
                    .get_asset_prices_callback()
            )
    }

    #[private]
    pub fn get_asset_prices_callback(
        &self,
        #[callback_result] call_result: Result<OraclePriceData, PromiseError>,
    ) -> HashMap<String, (u128, u32)> {
        let prices = match call_result {
            Ok(data) => data,
            Err(_) => env::panic_str("Failed to fetch price data from oracle"),
        };

        let timestamp = prices.timestamp.parse::<u64>().unwrap_or(0);
        let current_time = env::block_timestamp();
        assert!(
            current_time - timestamp < prices.recency_duration_sec * 1_000_000_000,
            "Price data is too old"
        );

        let mut asset_prices = HashMap::new();
        for price_data in prices.prices {
            if let Some(price) = price_data.price {
                asset_prices.insert(
                    price_data.asset_id,
                    (
                        price.multiplier.parse().unwrap_or(0),
                        price.decimals
                    )
                );
            }
        }

        let mut result = HashMap::new();
        for asset in &self.assets {
            if let Some(&near_address) = TOKEN_ADDRESSES.get(asset.contract_address.to_lowercase().as_str()) {
                if let Some(&(multiplier, decimals)) = asset_prices.get(near_address) {
                    result.insert(asset.contract_address.clone(), (multiplier, decimals));
                }
            }
        }

        result
    }

    #[private]
    pub fn process_deposit(&mut self, sender_id: AccountId, amount: U128) {
        self.get_asset_prices()
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(50))
                    .process_deposit_with_prices(sender_id, amount)
            );
    }

    #[private]
    pub fn process_deposit_with_prices(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        #[callback_result] prices_result: Result<HashMap<String, (u128, u32)>, PromiseError>,
    ) {
        let asset_prices = match prices_result {
            Ok(prices) => prices,
            Err(_) => env::panic_str("Failed to fetch asset prices"),
        };

        let user_balance = self
            .user_balances
            .entry(sender_id.clone())
            .or_insert_with(HashMap::new);

        for asset in &self.assets {
            if let Some(&(multiplier, decimals)) = asset_prices.get(&asset.contract_address) {
                let price = (multiplier as f64) / 10_u64.pow(decimals) as f64;
                let weight_fraction = f64::from(asset.weight) / 100.0;
                let asset_amount = (amount.0 as f64 * weight_fraction / price) as u128;

                user_balance
                    .entry(asset.contract_address.clone())
                    .and_modify(|balance| *balance = U128(balance.0 + asset_amount))
                    .or_insert(U128(asset_amount));
            }
        }

        self.total_assets = U128(self.total_assets.0 + amount.0);

        env::log_str(&format!(
            "Processed deposit for user {} with amount {} USDC",
            sender_id, amount.0
        ));
    }

    #[payable]
    pub fn withdraw_in_usdc(&mut self, amount: U128) -> Promise {
        let sender_id = env::predecessor_account_id();
        assert!(
            self.user_balances.contains_key(&sender_id),
            "No balance found for user"
        );
        
        Promise::new(self.usdc_contract.clone())
            .function_call(
                "ft_transfer".to_string(),
                format!(
                    r#"{{"receiver_id": "{}", "amount": "{}"}}"#,
                    sender_id.clone(),
                    amount.0
                )
                .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(10)
            )
    }

    #[payable]
    pub fn withdraw_underlying_assets(&mut self, amounts: HashMap<String, U128>) -> Vec<Promise> {
        let sender_id = env::predecessor_account_id();
        let user_balances = self.user_balances.get(&sender_id)
            .expect("No balance found for user");

        let mut promises = Vec::new();
        let gas_per_promise = Gas::from_tgas(10);

        for asset in &self.assets {
            if let Some(amount) = amounts.get(&asset.contract_address) {
                let current_balance = user_balances.get(&asset.contract_address)
                    .expect("No balance found for this asset");
                assert!(current_balance.0 >= amount.0, "Insufficient balance");

                if let Some(&near_address) = TOKEN_ADDRESSES.get(asset.contract_address.to_lowercase().as_str()) {
                    promises.push(
                        Promise::new(near_address.parse().unwrap())
                            .function_call(
                                "ft_transfer".to_string(),
                                format!(
                                    r#"{{"receiver_id": "{}", "amount": "{}"}}"#,
                                    sender_id.clone(),
                                    amount.0
                                )
                                .into_bytes(),
                                NearToken::from_yoctonear(1),
                                gas_per_promise
                            )
                    );
                }
            }
        }

        if let Some(user_balances) = self.user_balances.get_mut(&sender_id) {
            for (token_address, amount) in amounts {
                if let Some(balance) = user_balances.get_mut(&token_address) {
                    balance.0 -= amount.0;
                }
            }
        }

        promises
    }
}

#[near_bindgen]
impl FungibleTokenReceiver for Contract {
    fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        assert_eq!(
            env::predecessor_account_id(),
            self.usdc_contract,
            "Only USDC token is accepted"
        );

        env::log_str(&format!("Received {} USDC from {}", amount.0, sender_id));

        if msg.is_empty() {
            self.process_deposit(sender_id, amount);
            PromiseOrValue::Value(U128(0))
        } else {
            env::log_str(&format!("Unsupported message: {}", msg));
            PromiseOrValue::Value(amount)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    fn get_context(predecessor: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder.predecessor_account_id(predecessor);
        builder
    }

    #[test]
    fn test_new() {
        let context = get_context(accounts(1));
        testing_env!(context.build());

        let assets = vec![
            AssetInfo {
                name: "ETH".to_string(),
                contract_address: "0x2e5221B0f855Be4ea5Cefffb8311EED0563B6e87".to_string(),
                weight: 70,
            },
            AssetInfo {
                name: "AURORA".to_string(),
                contract_address: "0xe09D8aDae1141181f4CddddeF97E4Cf68f5436E6".to_string(),
                weight: 30,
            },
        ];

        let contract = Contract::new(
            accounts(1),
            assets.clone(),
            "3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af".parse().unwrap(),
        );
        
        assert_eq!(contract.get_number_of_assets(), 2);
        assert_eq!(contract.get_assets(), assets);
    }
}