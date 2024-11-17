use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    env, near_bindgen, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
    PromiseOrValue,
};
use omni_transaction::evm::evm_transaction::EVMTransaction;
use omni_transaction::evm::utils::parse_eth_address;
use omni_transaction::transaction_builder::{TransactionBuilder, TxBuilder};
use omni_transaction::types::EVM;
use once_cell::sync::Lazy;
use std::collections::HashMap;

// Constants
const MPC_CONTRACT_ACCOUNT_ID: &str = "v1.signer-prod.testnet";
const ETH_TREASURY_PATH: &str = "eth-treasury";
const AURORA_TREASURY_PATH: &str = "aurora-treasury";

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct WithdrawRequest {
    pub eth_destination: String,
    pub aurora_destination: String,
    pub network_details: NetworkDetails,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct NetworkDetails {
    pub chain_id: u64,
    pub eth_nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u128,
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
    pub latest_signed_txs: Vec<Vec<u8>>,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId, assets: Vec<AssetInfo>) -> Self {
        assert!(!env::state_exists(), "Contract is already initialized");
        let total_weight: u8 = assets.iter().map(|a| a.weight).sum();
        assert_eq!(total_weight, 100, "Total weight of assets must equal 100%");

        Self {
            total_assets: U128(0),
            assets,
            owner_id,
            user_balances: HashMap::new(),
            usdc_contract: "3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af"
                .parse::<AccountId>()
                .unwrap(),
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
                Gas::from_tgas(100),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(50))
                    .get_asset_prices_callback(),
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
                    (price.multiplier.parse().unwrap_or(0), price.decimals),
                );
            }
        }

        let mut result = HashMap::new();
        for asset in &self.assets {
            if let Some(&near_address) =
                TOKEN_ADDRESSES.get(asset.contract_address.to_lowercase().as_str())
            {
                if let Some(&(multiplier, decimals)) = asset_prices.get(near_address) {
                    result.insert(asset.contract_address.clone(), (multiplier, decimals));
                }
            }
        }

        result
    }

    #[private]
    pub fn process_deposit(&mut self, sender_id: AccountId, amount: U128) {
        self.get_asset_prices().then(
            Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(50))
                .process_deposit_with_prices(sender_id, amount),
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

        Promise::new(self.usdc_contract.clone()).function_call(
            "ft_transfer".to_string(),
            format!(
                r#"{{"receiver_id": "{}", "amount": "{}"}}"#,
                sender_id.clone(),
                amount.0
            )
            .into_bytes(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(10),
        )
    }

    #[payable]
    pub fn withdraw_underlying_assets(&mut self, request: WithdrawRequest) -> Promise {
        let sender_id = env::predecessor_account_id();
        let user_balances = self
            .user_balances
            .get(&sender_id)
            .expect("No balance found for user");

        // Create transactions for each asset type
        for asset in &self.assets {
            if let Some(balance) = user_balances.get(&asset.contract_address) {
                if balance.0 > 0 {
                    let destination = if asset.name == "ETH" {
                        request.eth_destination.clone()
                    } else {
                        request.aurora_destination.clone()
                    };

                    // Construct and sign the transaction
                    self.create_and_sign_withdrawal(
                        asset.contract_address.clone(),
                        destination,
                        balance.0,
                        request.network_details.clone(),
                        if asset.name == "ETH" {
                            ETH_TREASURY_PATH
                        } else {
                            AURORA_TREASURY_PATH
                        },
                    );
                }
            }
        }

        // Clear balances after initiating withdrawals
        if let Some(user_balances) = self.user_balances.get_mut(&sender_id) {
            for asset in &self.assets {
                if let Some(balance) = user_balances.get_mut(&asset.contract_address) {
                    balance.0 = 0;
                }
            }
        }

        Promise::new(env::current_account_id())
    }

    #[private]
    fn create_and_sign_withdrawal(
        &mut self,
        token_address: String,
        recipient: String,
        amount: u128,
        network_details: NetworkDetails,
        treasury_path: &str,
    ) -> Promise {
        let omni_tx = self.construct_erc20_transfer_tx(
            token_address,
            recipient,
            amount,
            network_details,
        );

        // Encode and hash the transaction
        let encoded_tx = omni_tx.build_for_signing();
        let tx_hash = env::keccak256(&encoded_tx);

        // Create the signing request
        let sign_request = SignRequest {
            payload: tx_hash.to_vec(),
            path: treasury_path.to_string(),
            key_version: 0,
        };

        // Send to MPC signer
        mpc::ext(MPC_CONTRACT_ACCOUNT_ID.parse().unwrap())
            .with_static_gas(Gas::from_tgas(100))
            .with_attached_deposit(NearToken::from_yoctonear(200000000000000000000000))
            .sign(sign_request)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(5))
                    .sign_callback(EVMTransactionWrapper::from_evm_transaction(&omni_tx))
            )
    }

    fn construct_erc20_transfer_tx(
        &self,
        token_address: String,
        recipient_address: String,
        amount: u128,
        network_details: NetworkDetails,
    ) -> EVMTransaction {
        let token_address = parse_eth_address(&token_address);
        let recipient_address = parse_eth_address(&recipient_address);

        let data = self.construct_erc20_transfer_data(recipient_address, amount);

        TransactionBuilder::new::<EVM>()
            .nonce(network_details.eth_nonce)
            .to(token_address)
            .value(0)
            .input(data)
            .max_priority_fee_per_gas(network_details.max_priority_fee_per_gas)
            .max_fee_per_gas(network_details.max_fee_per_gas)
            .gas_limit(network_details.gas_limit)
            .chain_id(network_details.chain_id)
            .build()
    }

    fn construct_erc20_transfer_data(&self, to: [u8; 20], amount: u128) -> Vec<u8> {
        let mut data = Vec::new();
        // Function selector for "transfer(address,uint256)"
        data.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]);
        // Pad the 'to' address to 32 bytes
        data.extend_from_slice(&[0; 12]);
        data.extend_from_slice(&to);
        // Pad the amount to 32 bytes
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&amount.to_be_bytes());
        data
    }

    #[private]
    pub fn sign_callback(
        &mut self,
        evm_tx_wrapper: EVMTransactionWrapper,
        #[callback_result] result: Result<SignResult, PromiseError>,
    ) -> Vec<u8> {
        let mpc_signature = result.unwrap();
        let big_r = &mpc_signature.big_r.affine_point;
        let s = &mpc_signature.s.scalar;

        let r = &big_r[2..];
        let v = mpc_signature.recovery_id;
        let signature_omni = OmniSignature {
            v,
            r: hex::decode(r).unwrap(),
            s: hex::decode(s).unwrap(),
        };

        let evm_tx = evm_tx_wrapper.to_evm_transaction();
        let signed_tx = evm_tx.build_with_signature(&signature_omni);
        
        self.latest_signed_txs.push(signed_tx.clone());
        signed_tx
    }

    // View method to get latest signed transactions
    pub fn get_latest_signed_txs(&self) -> Vec<Vec<u8>> {
        self.latest_signed_txs.clone()
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
            "3e2210e1184b45b64c8a434c0a7e7b23cc04ea7eb7a6c3c32520d03d4afcb8af"
                .parse()
                .unwrap(),
        );

        assert_eq!(contract.get_number_of_assets(), 2);
        assert_eq!(contract.get_assets(), assets);
    }
}
