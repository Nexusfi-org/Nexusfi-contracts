use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, Vector};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{env, log, near_bindgen, AccountId, PanicOnDefault, Promise};
use pyth::state::{Price, PriceIdentifier}; // Import Pyth price feed types.

#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetInfo {
    pub name: String,
    pub contract_address: AccountId,
    pub weight: u8,
    pub price_identifier: PriceIdentifier, // Add Pyth price identifier for the asset.
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct IndexFundToken {
    pub total_assets: U128,
    pub assets: Vector<AssetInfo>,
    pub owner_id: AccountId,
    pub user_balances: LookupMap<AccountId, LookupMap<String, U128>>,
    pub pyth_account: AccountId, // Pyth contract account for fetching prices.
}

#[near_bindgen]
impl IndexFundToken {
    #[init]
    pub fn new(owner_id: AccountId, pyth_account: AccountId) -> Self {
        assert!(!env::state_exists(), "Contract is already initialized");
        Self {
            total_assets: U128(0),
            assets: Vector::new(b"a"),
            owner_id,
            user_balances: LookupMap::new(b"u"),
            pyth_account,
        }
    }

    #[payable]
    pub fn deposit_usdc(&mut self, usdc_amount: U128) -> Promise {
        let user = env::predecessor_account_id();
        let usdc_value = usdc_amount.0;

        log!("User {} deposited {} USDC", user, usdc_value);

        // Fetch prices of all underlying assets.
        let price_promises: Vec<_> = self
            .assets
            .iter()
            .map(|asset| {
                pyth::ext::ext_pyth::ext(self.pyth_account.clone())
                    .with_static_gas(env::prepaid_gas() / self.assets.len() as u64)
                    .get_price(asset.price_identifier.clone())
            })
            .collect();

        // Combine all price fetches into a single promise.
        Promise::all(price_promises).then(
            Self::ext(env::current_account_id())
                .with_static_gas(env::prepaid_gas() / 4)
                .allocate_assets(user, usdc_amount),
        )
    }

    #[private]
    pub fn allocate_assets(
        &mut self,
        user: AccountId,
        usdc_amount: U128,
        #[callback_result] results: Vec<Result<Price, PromiseError>>,
    ) {
        let mut user_allocation = LookupMap::new(format!("alloc:{}", user).as_bytes());
        let usdc_value = usdc_amount.0;

        for (asset, price_result) in self.assets.iter().zip(results) {
            if let Ok(price) = price_result {
                let asset_quantity =
                    usdc_value * u128::from(asset.weight) / (price.price as u128);
                user_allocation.insert(&asset.name, &U128(asset_quantity));
                log!(
                    "Allocated {} of {} for user {}",
                    asset_quantity,
                    asset.name,
                    user
                );
            } else {
                log!("Failed to fetch price for asset {}", asset.name);
            }
        }

        self.user_balances.insert(&user, &user_allocation);
    }

    pub fn withdraw_in_usdc(&mut self) -> Promise {
        let user = env::predecessor_account_id();
        let user_allocation = self.user_balances.get(&user).expect("No allocations found");

        let price_promises: Vec<_> = self
            .assets
            .iter()
            .map(|asset| {
                pyth::ext::ext_pyth::ext(self.pyth_account.clone())
                    .with_static_gas(env::prepaid_gas() / self.assets.len() as u64)
                    .get_price(asset.price_identifier.clone())
            })
            .collect();

        Promise::all(price_promises).then(
            Self::ext(env::current_account_id())
                .with_static_gas(env::prepaid_gas() / 4)
                .calculate_usdc_value(user_allocation, user),
        )
    }

    #[private]
    pub fn calculate_usdc_value(
        &self,
        user_allocation: LookupMap<String, U128>,
        user: AccountId,
        #[callback_result] results: Vec<Result<Price, PromiseError>>,
    ) {
        let mut total_usdc_value = 0u128;

        for (asset, price_result) in self.assets.iter().zip(results) {
            if let Ok(price) = price_result {
                if let Some(quantity) = user_allocation.get(&asset.name) {
                    total_usdc_value += quantity.0 * (price.price as u128);
                }
            } else {
                log!("Failed to fetch price for asset {}", asset.name);
            }
        }

        log!(
            "Withdrawing {} USDC for user {}",
            total_usdc_value,
            user
        );

        Promise::new(user).transfer(total_usdc_value);
    }

    fn assert_owner(&self) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_id,
            "Only the owner can call this method"
        );
    }
}
