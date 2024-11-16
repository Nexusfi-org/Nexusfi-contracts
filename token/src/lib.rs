use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::{env, near_bindgen, AccountId, PanicOnDefault, Promise, PromiseOrValue, NearToken, Gas};
use std::collections::HashMap;

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: AccountId,
    pub weight: u8,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub total_assets: U128,
    pub assets: Vec<AssetInfo>,
    pub owner_id: AccountId,
    pub user_balances: HashMap<AccountId, HashMap<String, U128>>,
    pub usdc_contract: AccountId,
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

    fn get_asset_prices(&self) -> HashMap<String, f64> {
        env::log_str("Fetching asset prices... (placeholder)");
        self.assets
            .iter()
            .map(|asset| (asset.name.clone(), 1.0))
            .collect()
    }

    #[private]
    pub fn process_deposit(&mut self, sender_id: AccountId, amount: U128) {
        let asset_prices = self.get_asset_prices();
        let user_balance = self
            .user_balances
            .entry(sender_id.clone())
            .or_insert_with(HashMap::new);

        for asset in &self.assets {
            let price = asset_prices.get(&asset.name).expect("Missing asset price");
            let weight_fraction = f64::from(asset.weight) / 100.0;
            let asset_amount = (amount.0 as f64 * weight_fraction / price) as u128;

            user_balance
                .entry(asset.name.clone())
                .and_modify(|balance| *balance = U128(balance.0 + asset_amount))
                .or_insert(U128(asset_amount));
        }

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
        assert!(
            self.user_balances.contains_key(&sender_id),
            "No balance found for user"
        );
        
        let gas_per_promise = Gas::from_tgas(10);
        
        self.assets
            .iter()
            .filter_map(|asset| {
                amounts.get(&asset.name).map(|amount| {
                    Promise::new(asset.contract_address.clone())
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
                })
            })
            .collect()
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
            PromiseOrValue::Value(amount) // Refund if message is not empty
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
                contract_address: accounts(2),
                weight: 70,
            },
            AssetInfo {
                name: "BTC".to_string(),
                contract_address: accounts(3),
                weight: 30,
            },
        ];

        let contract = Contract::new(accounts(1), assets.clone(), accounts(4));
        assert_eq!(contract.get_number_of_assets(), 2);
        assert_eq!(contract.get_assets(), assets);
    }
}