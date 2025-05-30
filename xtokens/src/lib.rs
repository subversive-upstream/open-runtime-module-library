//! # Xtokens Module
//!
//! ## Overview
//!
//! The xtokens module provides cross-chain token transfer functionality, by
//! cross-consensus messages(XCM).
//!
//! The xtokens module provides functions for
//! - Token transfer from parachains to relay chain.
//! - Token transfer between parachains, including relay chain tokens like DOT,
//!   KSM, and parachain tokens like ACA, aUSD.
//!
//! ## Interface
//!
//! ### Dispatchable functions
//!
//! - `transfer`: Transfer local assets with given `CurrencyId` and `Amount`.
//! - `transfer_multiasset`: Transfer `Asset` assets.
//! - `transfer_with_fee`: Transfer native currencies specifying the fee and
//!   amount as separate.
//! - `transfer_multiasset_with_fee`: Transfer `Asset` specifying the fee and
//!   amount as separate.
//! - `transfer_multicurrencies`: Transfer several currencies specifying the
//!   item to be used as fee.
//! - `transfer_multiassets`: Transfer several `Asset` specifying the item to be
//!   used as fee.

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::from_over_into)]
#![allow(clippy::unused_unit)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::boxed_local)]
#![allow(clippy::too_many_arguments)]

use frame_support::{
	pallet_prelude::*,
	require_transactional,
	traits::{Contains, Get},
	Parameter,
};
use frame_system::{ensure_signed, pallet_prelude::*};
use sp_runtime::{
	traits::{AtLeast32BitUnsigned, Bounded, Convert, MaybeSerializeDeserialize, Member, Zero},
	DispatchError,
};
use sp_std::{prelude::*, result::Result};

use xcm::{
	v5::{prelude::*, Weight},
	VersionedAsset, VersionedAssets, VersionedLocation,
};
use xcm_executor::traits::WeightBounds;

pub use module::*;
use orml_traits::{
	location::{Parse, Reserve},
	xcm_transfer::{Transferred, XtokensWeightInfo},
	GetByKey, RateLimiter, XcmTransfer,
};

mod mock;
mod tests;

enum TransferKind {
	/// Transfer self reserve asset.
	SelfReserveAsset,
	/// To reserve location.
	ToReserve,
	/// To non-reserve location.
	ToNonReserve,
}
use TransferKind::*;

#[frame_support::pallet]
pub mod module {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The balance type.
		type Balance: Parameter
			+ Member
			+ AtLeast32BitUnsigned
			+ Default
			+ Copy
			+ MaybeSerializeDeserialize
			+ Into<u128>;

		/// Currency Id.
		type CurrencyId: Parameter + Member + Clone;

		/// Convert `T::CurrencyId` to `Location`.
		type CurrencyIdConvert: Convert<Self::CurrencyId, Option<Location>>;

		/// Convert `T::AccountId` to `Location`.
		type AccountIdToLocation: Convert<Self::AccountId, Location>;

		/// Self chain location.
		#[pallet::constant]
		type SelfLocation: Get<Location>;

		/// Minimum xcm execution fee paid on destination chain.
		type MinXcmFee: GetByKey<Location, Option<u128>>;

		/// XCM executor.
		type XcmExecutor: ExecuteXcm<Self::RuntimeCall>;

		/// Location filter
		type LocationsFilter: Contains<Location>;

		/// Means of measuring the weight consumed by an XCM message locally.
		type Weigher: WeightBounds<Self::RuntimeCall>;

		/// Base XCM weight.
		///
		/// The actually weight for an XCM message is `T::BaseXcmWeight +
		/// T::Weigher::weight(&msg)`.
		#[pallet::constant]
		type BaseXcmWeight: Get<Weight>;

		/// This chain's Universal Location.
		type UniversalLocation: Get<InteriorLocation>;

		/// The maximum number of distinct assets allowed to be transferred in a
		/// single helper extrinsic.
		type MaxAssetsForTransfer: Get<usize>;

		/// The way to retreave the reserve of a Asset. This can be
		/// configured to accept absolute or relative paths for self tokens
		type ReserveProvider: Reserve;

		/// The rate limiter used to limit the cross-chain transfer asset.
		type RateLimiter: RateLimiter;

		/// The id of the RateLimiter.
		#[pallet::constant]
		type RateLimiterId: Get<<Self::RateLimiter as RateLimiter>::RateLimiterId>;
	}

	#[pallet::event]
	#[pallet::generate_deposit(fn deposit_event)]
	pub enum Event<T: Config> {
		/// Transferred `Asset` with fee.
		TransferredAssets {
			sender: T::AccountId,
			assets: Assets,
			fee: Asset,
			dest: Location,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Asset has no reserve location.
		AssetHasNoReserve,
		/// Not cross-chain transfer.
		NotCrossChainTransfer,
		/// Invalid transfer destination.
		InvalidDest,
		/// Currency is not cross-chain transferable.
		NotCrossChainTransferableCurrency,
		/// The message's weight could not be determined.
		UnweighableMessage,
		/// XCM execution failed.
		XcmExecutionFailed,
		/// Could not re-anchor the assets to declare the fees for the
		/// destination chain.
		CannotReanchor,
		/// Could not get ancestry of asset reserve location.
		InvalidAncestry,
		/// The Asset is invalid.
		InvalidAsset,
		/// The destination `Location` provided cannot be inverted.
		DestinationNotInvertible,
		/// The version of the `Versioned` value used is not able to be
		/// interpreted.
		BadVersion,
		/// We tried sending distinct asset and fee but they have different
		/// reserve chains.
		DistinctReserveForAssetAndFee,
		/// The fee is zero.
		ZeroFee,
		/// The transferring asset amount is zero.
		ZeroAmount,
		/// The number of assets to be sent is over the maximum.
		TooManyAssetsBeingSent,
		/// The specified index does not exist in a Assets struct.
		AssetIndexNonExistent,
		/// Fee is not enough.
		FeeNotEnough,
		/// Not supported Location
		NotSupportedLocation,
		/// MinXcmFee not registered for certain reserve location
		MinXcmFeeNotDefined,
		/// Asset transfer is limited by RateLimiter.
		RateLimited,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Transfer native currencies.
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(0)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer(currency_id.clone(), *amount, dest))]
		pub fn transfer(
			origin: OriginFor<T>,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;
			Self::do_transfer(who, currency_id, amount, dest, dest_weight_limit).map(|_| ())
		}

		/// Transfer `Asset`.
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(1)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer_multiasset(asset, dest))]
		pub fn transfer_multiasset(
			origin: OriginFor<T>,
			asset: Box<VersionedAsset>,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let asset: Asset = (*asset).try_into().map_err(|()| Error::<T>::BadVersion)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;
			Self::do_transfer_asset(who, asset, dest, dest_weight_limit).map(|_| ())
		}

		/// Transfer native currencies specifying the fee and amount as
		/// separate.
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// `fee` is the amount to be spent to pay for execution in destination
		/// chain. Both fee and amount will be subtracted form the callers
		/// balance.
		///
		/// If `fee` is not high enough to cover for the execution costs in the
		/// destination chain, then the assets will be trapped in the
		/// destination chain
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(2)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer(currency_id.clone(), *amount, dest))]
		pub fn transfer_with_fee(
			origin: OriginFor<T>,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			fee: T::Balance,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;

			Self::do_transfer_with_fee(who, currency_id, amount, fee, dest, dest_weight_limit).map(|_| ())
		}

		/// Transfer `Asset` specifying the fee and amount as separate.
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// `fee` is the Asset to be spent to pay for execution in
		/// destination chain. Both fee and amount will be subtracted form the
		/// callers balance For now we only accept fee and asset having the same
		/// `Location` id.
		///
		/// If `fee` is not high enough to cover for the execution costs in the
		/// destination chain, then the assets will be trapped in the
		/// destination chain
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(3)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer_multiasset(asset, dest))]
		pub fn transfer_multiasset_with_fee(
			origin: OriginFor<T>,
			asset: Box<VersionedAsset>,
			fee: Box<VersionedAsset>,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let asset: Asset = (*asset).try_into().map_err(|()| Error::<T>::BadVersion)?;
			let fee: Asset = (*fee).try_into().map_err(|()| Error::<T>::BadVersion)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;

			Self::do_transfer_asset_with_fee(who, asset, fee, dest, dest_weight_limit).map(|_| ())
		}

		/// Transfer several currencies specifying the item to be used as fee
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// `fee_item` is index of the currencies tuple that we want to use for
		/// payment
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(4)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer_multicurrencies(currencies, fee_item, dest))]
		pub fn transfer_multicurrencies(
			origin: OriginFor<T>,
			currencies: Vec<(T::CurrencyId, T::Balance)>,
			fee_item: u32,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;

			Self::do_transfer_multicurrencies(who, currencies, fee_item, dest, dest_weight_limit).map(|_| ())
		}

		/// Transfer several `Asset` specifying the item to be used as fee
		///
		/// `dest_weight_limit` is the weight for XCM execution on the dest
		/// chain, and it would be charged from the transferred assets. If set
		/// below requirements, the execution may fail and assets wouldn't be
		/// received.
		///
		/// `fee_item` is index of the Assets that we want to use for
		/// payment
		///
		/// It's a no-op if any error on local XCM execution or message sending.
		/// Note sending assets out per se doesn't guarantee they would be
		/// received. Receiving depends on if the XCM message could be delivered
		/// by the network, and if the receiving chain would handle
		/// messages correctly.
		#[pallet::call_index(5)]
		#[pallet::weight(XtokensWeight::<T>::weight_of_transfer_multiassets(assets, fee_item, dest))]
		pub fn transfer_multiassets(
			origin: OriginFor<T>,
			assets: Box<VersionedAssets>,
			fee_item: u32,
			dest: Box<VersionedLocation>,
			dest_weight_limit: WeightLimit,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let assets: Assets = (*assets).try_into().map_err(|()| Error::<T>::BadVersion)?;
			let dest: Location = (*dest).try_into().map_err(|()| Error::<T>::BadVersion)?;

			// We first grab the fee
			let fee: &Asset = assets.get(fee_item as usize).ok_or(Error::<T>::AssetIndexNonExistent)?;

			Self::do_transfer_assets(who, assets.clone(), fee.clone(), dest, dest_weight_limit).map(|_| ())
		}
	}

	impl<T: Config> Pallet<T> {
		fn do_transfer(
			who: T::AccountId,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			let location: Location =
				T::CurrencyIdConvert::convert(currency_id).ok_or(Error::<T>::NotCrossChainTransferableCurrency)?;

			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);
			ensure!(T::LocationsFilter::contains(&dest), Error::<T>::NotSupportedLocation);

			let asset: Asset = (location, amount.into()).into();
			Self::do_transfer_assets(who, vec![asset.clone()].into(), asset, dest, dest_weight_limit)
		}

		fn do_transfer_with_fee(
			who: T::AccountId,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			fee: T::Balance,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			let location: Location =
				T::CurrencyIdConvert::convert(currency_id).ok_or(Error::<T>::NotCrossChainTransferableCurrency)?;

			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);
			ensure!(!fee.is_zero(), Error::<T>::ZeroFee);
			ensure!(T::LocationsFilter::contains(&dest), Error::<T>::NotSupportedLocation);

			let asset = (location.clone(), amount.into()).into();
			let fee_asset: Asset = (location, fee.into()).into();

			// Push contains saturated addition, so we should be able to use it safely
			let mut assets = Assets::new();
			assets.push(asset);
			assets.push(fee_asset.clone());

			Self::do_transfer_assets(who, assets, fee_asset, dest, dest_weight_limit)
		}

		fn do_transfer_asset(
			who: T::AccountId,
			asset: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_assets(who, vec![asset.clone()].into(), asset, dest, dest_weight_limit)
		}

		fn do_transfer_asset_with_fee(
			who: T::AccountId,
			asset: Asset,
			fee: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			// Push contains saturated addition, so we should be able to use it safely
			let mut assets = Assets::new();
			assets.push(asset);
			assets.push(fee.clone());

			Self::do_transfer_assets(who, assets, fee, dest, dest_weight_limit)
		}

		fn do_transfer_multicurrencies(
			who: T::AccountId,
			currencies: Vec<(T::CurrencyId, T::Balance)>,
			fee_item: u32,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			ensure!(
				currencies.len() <= T::MaxAssetsForTransfer::get(),
				Error::<T>::TooManyAssetsBeingSent
			);
			ensure!(T::LocationsFilter::contains(&dest), Error::<T>::NotSupportedLocation);

			let mut assets = Assets::new();

			// Lets grab the fee amount and location first
			let (fee_currency_id, fee_amount) = currencies
				.get(fee_item as usize)
				.ok_or(Error::<T>::AssetIndexNonExistent)?;

			for (currency_id, amount) in &currencies {
				let location: Location = T::CurrencyIdConvert::convert(currency_id.clone())
					.ok_or(Error::<T>::NotCrossChainTransferableCurrency)?;
				ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);

				// Push contains saturated addition, so we should be able to use it safely
				assets.push((location, (*amount).into()).into())
			}

			// We construct the fee now, since getting it from assets won't work as assets
			// sorts it
			let fee_location: Location = T::CurrencyIdConvert::convert(fee_currency_id.clone())
				.ok_or(Error::<T>::NotCrossChainTransferableCurrency)?;

			let fee: Asset = (fee_location, (*fee_amount).into()).into();

			Self::do_transfer_assets(who, assets, fee, dest, dest_weight_limit)
		}

		fn do_transfer_assets(
			who: T::AccountId,
			assets: Assets,
			fee: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			ensure!(
				assets.len() <= T::MaxAssetsForTransfer::get(),
				Error::<T>::TooManyAssetsBeingSent
			);
			ensure!(T::LocationsFilter::contains(&dest), Error::<T>::NotSupportedLocation);

			// Fee payment can only be made by using the non-zero amount of fungibles
			ensure!(
				matches!(fee.fun, Fungibility::Fungible(x) if !x.is_zero()),
				Error::<T>::InvalidAsset
			);

			let origin_location = T::AccountIdToLocation::convert(who.clone());

			let mut non_fee_reserve: Option<Location> = None;
			let asset_len = assets.len();
			for i in 0..asset_len {
				let asset = assets.get(i).ok_or(Error::<T>::AssetIndexNonExistent)?;

				match asset.fun {
					Fungibility::Fungible(x) => ensure!(!x.is_zero(), Error::<T>::InvalidAsset),
					Fungibility::NonFungible(AssetInstance::Undefined) => return Err(Error::<T>::InvalidAsset.into()),
					_ => {}
				}

				// `assets` includes fee, the reserve location is decided by non fee asset
				if non_fee_reserve.is_none() && asset.id != fee.id {
					non_fee_reserve = T::ReserveProvider::reserve(asset);
				}

				// make sure all non fee assets share the same reserve
				if non_fee_reserve.is_some() {
					ensure!(
						non_fee_reserve == T::ReserveProvider::reserve(asset),
						Error::<T>::DistinctReserveForAssetAndFee
					);
				}

				// per asset check
				let amount = match asset.fun {
					Fungibility::Fungible(amount) => amount,
					Fungibility::NonFungible(_) => 1,
				};

				let rate_limiter_id = T::RateLimiterId::get();

				// try consume quota of the rate limiter.
				T::RateLimiter::try_consume(rate_limiter_id, asset.id.clone(), amount, Some(&who))
					.map_err(|_| Error::<T>::RateLimited)?;
			}

			let fee_reserve = T::ReserveProvider::reserve(&fee);
			if asset_len > 1 && fee_reserve != non_fee_reserve {
				// Current only support `ToReserve` with relay-chain asset as fee. other case
				// like `NonReserve` or `SelfReserve` with relay-chain fee is not support.
				ensure!(non_fee_reserve == dest.chain_part(), Error::<T>::InvalidAsset);

				let reserve_location = non_fee_reserve.clone().ok_or(Error::<T>::AssetHasNoReserve)?;
				let min_xcm_fee = T::MinXcmFee::get(&reserve_location).ok_or(Error::<T>::MinXcmFeeNotDefined)?;

				// min xcm fee should less than user fee
				let fee_to_dest: Asset = (fee.id.clone(), min_xcm_fee).into();
				ensure!(fee_to_dest < fee, Error::<T>::FeeNotEnough);

				let mut assets_to_dest = Assets::new();
				for i in 0..asset_len {
					let asset = assets.get(i).ok_or(Error::<T>::AssetIndexNonExistent)?;
					if fee != *asset {
						assets_to_dest.push(asset.clone());
					} else {
						assets_to_dest.push(fee_to_dest.clone());
					}
				}

				let mut assets_to_fee_reserve = Assets::new();
				let asset_to_fee_reserve = subtract_fee(&fee, min_xcm_fee);
				assets_to_fee_reserve.push(asset_to_fee_reserve.clone());

				let mut override_recipient = T::SelfLocation::get();
				if override_recipient == Location::here() {
					let dest_chain_part = dest.chain_part().ok_or(Error::<T>::InvalidDest)?;
					let ancestry = T::UniversalLocation::get();
					let _ = override_recipient
						.reanchor(&dest_chain_part, &ancestry)
						.map_err(|_| Error::<T>::CannotReanchor);
				}

				// First xcm sent to fee reserve chain and routed to dest chain.
				// We can use `MinXcmFee` configuration to decide which target parachain use
				// teleport. But as current there's only one case which is Parachain send back
				// asset to Statemine/t, So we set `use_teleport` to always `true` in this case.
				Self::execute_and_send_reserve_kind_xcm(
					origin_location.clone(),
					assets_to_fee_reserve,
					asset_to_fee_reserve,
					fee_reserve,
					&dest,
					Some(override_recipient),
					dest_weight_limit.clone(),
					true,
				)?;

				// Second xcm send to dest chain.
				Self::execute_and_send_reserve_kind_xcm(
					origin_location,
					assets_to_dest,
					fee_to_dest,
					non_fee_reserve,
					&dest,
					None,
					dest_weight_limit,
					false,
				)?;
			} else {
				Self::execute_and_send_reserve_kind_xcm(
					origin_location,
					assets.clone(),
					fee.clone(),
					fee_reserve,
					&dest,
					None,
					dest_weight_limit,
					false,
				)?;
			}

			Self::deposit_event(Event::<T>::TransferredAssets {
				sender: who.clone(),
				assets: assets.clone(),
				fee: fee.clone(),
				dest: dest.clone(),
			});

			Ok(Transferred {
				sender: who,
				assets,
				fee,
				dest,
			})
		}

		/// Execute and send xcm with given assets and fee to dest chain or
		/// reserve chain.
		fn execute_and_send_reserve_kind_xcm(
			origin_location: Location,
			assets: Assets,
			fee: Asset,
			reserve: Option<Location>,
			dest: &Location,
			maybe_recipient_override: Option<Location>,
			dest_weight_limit: WeightLimit,
			use_teleport: bool,
		) -> DispatchResult {
			let (transfer_kind, dest, reserve, recipient) = Self::transfer_kind(reserve, dest)?;
			let recipient = match maybe_recipient_override {
				Some(recipient) => recipient,
				None => recipient,
			};
			let mut msg = match transfer_kind {
				SelfReserveAsset => Self::transfer_self_reserve_asset(assets, fee, dest, recipient, dest_weight_limit)?,
				ToReserve => Self::transfer_to_reserve(assets, fee, dest, recipient, dest_weight_limit)?,
				ToNonReserve => Self::transfer_to_non_reserve(
					assets,
					fee,
					reserve,
					dest,
					recipient,
					dest_weight_limit,
					use_teleport,
				)?,
			};
			let mut hash = msg.using_encoded(sp_io::hashing::blake2_256);

			let weight = T::Weigher::weight(&mut msg).map_err(|()| Error::<T>::UnweighableMessage)?;
			T::XcmExecutor::prepare_and_execute(origin_location, msg, &mut hash, weight, weight)
				.ensure_complete()
				.map_err(|error| {
					log::error!("Failed execute transfer message with {:?}", error);
					Error::<T>::XcmExecutionFailed
				})?;

			Ok(())
		}

		fn transfer_self_reserve_asset(
			assets: Assets,
			fee: Asset,
			dest: Location,
			recipient: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Xcm<T::RuntimeCall>, DispatchError> {
			Ok(Xcm(vec![
				SetFeesMode { jit_withdraw: true },
				TransferReserveAsset {
					assets: assets.clone(),
					dest: dest.clone(),
					xcm: Xcm(vec![
						Self::buy_execution(fee, &dest, dest_weight_limit)?,
						Self::deposit_asset(recipient, assets.len() as u32),
					]),
				},
			]))
		}

		fn transfer_to_reserve(
			assets: Assets,
			fee: Asset,
			reserve: Location,
			recipient: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Xcm<T::RuntimeCall>, DispatchError> {
			Ok(Xcm(vec![
				WithdrawAsset(assets.clone()),
				SetFeesMode { jit_withdraw: true },
				InitiateReserveWithdraw {
					assets: All.into(),
					reserve: reserve.clone(),
					xcm: Xcm(vec![
						Self::buy_execution(fee, &reserve, dest_weight_limit)?,
						Self::deposit_asset(recipient, assets.len() as u32),
					]),
				},
			]))
		}

		fn transfer_to_non_reserve(
			assets: Assets,
			fee: Asset,
			reserve: Location,
			dest: Location,
			recipient: Location,
			dest_weight_limit: WeightLimit,
			use_teleport: bool,
		) -> Result<Xcm<T::RuntimeCall>, DispatchError> {
			let mut reanchored_dest = dest.clone();
			if reserve == Location::parent() {
				if let (1, [Parachain(id)]) = dest.unpack() {
					reanchored_dest = Parachain(*id).into();
				}
			}

			let max_assets = assets.len() as u32;
			if !use_teleport {
				Ok(Xcm(vec![
					WithdrawAsset(assets),
					SetFeesMode { jit_withdraw: true },
					InitiateReserveWithdraw {
						assets: All.into(),
						reserve: reserve.clone(),
						xcm: Xcm(vec![
							Self::buy_execution(half(&fee), &reserve, dest_weight_limit.clone())?,
							DepositReserveAsset {
								assets: AllCounted(max_assets).into(),
								dest: reanchored_dest,
								xcm: Xcm(vec![
									Self::buy_execution(half(&fee), &dest, dest_weight_limit)?,
									Self::deposit_asset(recipient, max_assets),
								]),
							},
						]),
					},
				]))
			} else {
				Ok(Xcm(vec![
					WithdrawAsset(assets),
					SetFeesMode { jit_withdraw: true },
					InitiateReserveWithdraw {
						assets: All.into(),
						reserve: reserve.clone(),
						xcm: Xcm(vec![
							Self::buy_execution(half(&fee), &reserve, dest_weight_limit.clone())?,
							InitiateTeleport {
								assets: All.into(),
								dest: reanchored_dest,
								xcm: Xcm(vec![
									Self::buy_execution(half(&fee), &dest, dest_weight_limit)?,
									Self::deposit_asset(recipient, max_assets),
								]),
							},
						]),
					},
				]))
			}
		}

		fn deposit_asset(recipient: Location, max_assets: u32) -> Instruction<()> {
			DepositAsset {
				assets: AllCounted(max_assets).into(),
				beneficiary: recipient,
			}
		}

		fn buy_execution(
			asset: Asset,
			at: &Location,
			weight_limit: WeightLimit,
		) -> Result<Instruction<()>, DispatchError> {
			let ancestry = T::UniversalLocation::get();
			let fees = asset
				.reanchored(at, &ancestry)
				.map_err(|_| Error::<T>::CannotReanchor)?;

			Ok(BuyExecution { fees, weight_limit })
		}

		/// Ensure has the `dest` has chain part and recipient part.
		fn ensure_valid_dest(dest: &Location) -> Result<(Location, Location), DispatchError> {
			if let (Some(dest), Some(recipient)) = (dest.chain_part(), dest.non_chain_part()) {
				Ok((dest, recipient))
			} else {
				Err(Error::<T>::InvalidDest.into())
			}
		}

		/// Get the transfer kind.
		///
		/// Returns `Err` if `dest` combination doesn't make sense, or `reserve`
		/// is none, else returns a tuple of:
		/// - `transfer_kind`.
		/// - asset's `reserve` parachain or relay chain location,
		/// - `dest` parachain or relay chain location.
		/// - `recipient` location.
		fn transfer_kind(
			reserve: Option<Location>,
			dest: &Location,
		) -> Result<(TransferKind, Location, Location, Location), DispatchError> {
			let (dest, recipient) = Self::ensure_valid_dest(dest)?;

			let self_location = T::SelfLocation::get();
			ensure!(dest != self_location, Error::<T>::NotCrossChainTransfer);
			let reserve = reserve.ok_or(Error::<T>::AssetHasNoReserve)?;
			let transfer_kind = if reserve == self_location {
				SelfReserveAsset
			} else if reserve == dest {
				ToReserve
			} else {
				ToNonReserve
			};
			Ok((transfer_kind, dest, reserve, recipient))
		}

		/// Get reserve location by `assets` and `fee_item`. the `assets`
		/// includes fee asset and non fee asset. make sure assets have ge one
		/// asset. all non fee asset should share same reserve location.
		fn get_reserve_location(assets: &Assets, fee_item: &u32) -> Option<Location> {
			let reserve_idx = if assets.len() == 1 {
				0
			} else {
				(*fee_item == 0) as usize
			};
			let asset = assets.get(reserve_idx);
			asset.and_then(T::ReserveProvider::reserve)
		}
	}

	pub struct XtokensWeight<T>(PhantomData<T>);
	// weights
	impl<T: Config> XtokensWeightInfo<T::AccountId, T::Balance, T::CurrencyId> for XtokensWeight<T> {
		/// Returns weight of `transfer_multiasset` call.
		fn weight_of_transfer_multiasset(asset: &VersionedAsset, dest: &VersionedLocation) -> Weight {
			let asset: Result<Asset, _> = asset.clone().try_into();
			let dest = dest.clone().try_into();
			if let (Ok(asset), Ok(dest)) = (asset, dest) {
				if let Ok((transfer_kind, dest, _, reserve)) =
					Pallet::<T>::transfer_kind(T::ReserveProvider::reserve(&asset), &dest)
				{
					let mut msg = match transfer_kind {
						SelfReserveAsset => Xcm(vec![
							SetFeesMode { jit_withdraw: true },
							TransferReserveAsset {
								assets: vec![asset].into(),
								dest,
								xcm: Xcm(vec![]),
							},
						]),
						ToReserve | ToNonReserve => Xcm(vec![
							WithdrawAsset(Assets::from(asset)),
							SetFeesMode { jit_withdraw: true },
							InitiateReserveWithdraw {
								assets: All.into(),
								// `dest` is always (equal to) `reserve` in both cases
								reserve,
								xcm: Xcm(vec![]),
							},
						]),
					};
					return T::Weigher::weight(&mut msg)
						.map_or(Weight::max_value(), |w| T::BaseXcmWeight::get().saturating_add(w));
				}
			}
			Weight::zero()
		}

		/// Returns weight of `transfer` call.
		fn weight_of_transfer(currency_id: T::CurrencyId, amount: T::Balance, dest: &VersionedLocation) -> Weight {
			if let Some(location) = T::CurrencyIdConvert::convert(currency_id) {
				let asset = (location, amount.into()).into();
				Self::weight_of_transfer_multiasset(&asset, dest)
			} else {
				Weight::zero()
			}
		}

		/// Returns weight of `transfer` call.
		fn weight_of_transfer_multicurrencies(
			currencies: &[(T::CurrencyId, T::Balance)],
			fee_item: &u32,
			dest: &VersionedLocation,
		) -> Weight {
			let mut assets: Vec<Asset> = Vec::new();
			for (currency_id, amount) in currencies {
				if let Some(location) = T::CurrencyIdConvert::convert(currency_id.clone()) {
					let asset: Asset = (location, (*amount).into()).into();
					assets.push(asset);
				} else {
					return Weight::zero();
				}
			}

			Self::weight_of_transfer_multiassets(&VersionedAssets::from(Assets::from(assets)), fee_item, dest)
		}

		/// Returns weight of `transfer_multiassets` call.
		fn weight_of_transfer_multiassets(
			assets: &VersionedAssets,
			fee_item: &u32,
			dest: &VersionedLocation,
		) -> Weight {
			let assets: Result<Assets, ()> = assets.clone().try_into();
			let dest = dest.clone().try_into();
			if let (Ok(assets), Ok(dest)) = (assets, dest) {
				let reserve_location = Pallet::<T>::get_reserve_location(&assets, fee_item);
				if let Ok((transfer_kind, dest, _, reserve)) = Pallet::<T>::transfer_kind(reserve_location, &dest) {
					let mut msg = match transfer_kind {
						SelfReserveAsset => Xcm(vec![
							SetFeesMode { jit_withdraw: true },
							TransferReserveAsset {
								assets,
								dest,
								xcm: Xcm(vec![]),
							},
						]),
						ToReserve | ToNonReserve => Xcm(vec![
							WithdrawAsset(assets),
							SetFeesMode { jit_withdraw: true },
							InitiateReserveWithdraw {
								assets: All.into(),
								// `dest` is always (equal to) `reserve` in both cases
								reserve,
								xcm: Xcm(vec![]),
							},
						]),
					};
					return T::Weigher::weight(&mut msg)
						.map_or(Weight::max_value(), |w| T::BaseXcmWeight::get().saturating_add(w));
				}
			}
			Weight::zero()
		}
	}

	impl<T: Config> XcmTransfer<T::AccountId, T::Balance, T::CurrencyId> for Pallet<T> {
		#[require_transactional]
		fn transfer(
			who: T::AccountId,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer(who, currency_id, amount, dest, dest_weight_limit)
		}

		#[require_transactional]
		fn transfer_multiasset(
			who: T::AccountId,
			asset: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_asset(who, asset, dest, dest_weight_limit)
		}

		#[require_transactional]
		fn transfer_with_fee(
			who: T::AccountId,
			currency_id: T::CurrencyId,
			amount: T::Balance,
			fee: T::Balance,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_with_fee(who, currency_id, amount, fee, dest, dest_weight_limit)
		}

		#[require_transactional]
		fn transfer_multiasset_with_fee(
			who: T::AccountId,
			asset: Asset,
			fee: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_asset_with_fee(who, asset, fee, dest, dest_weight_limit)
		}

		#[require_transactional]
		fn transfer_multicurrencies(
			who: T::AccountId,
			currencies: Vec<(T::CurrencyId, T::Balance)>,
			fee_item: u32,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_multicurrencies(who, currencies, fee_item, dest, dest_weight_limit)
		}

		#[require_transactional]
		fn transfer_multiassets(
			who: T::AccountId,
			assets: Assets,
			fee: Asset,
			dest: Location,
			dest_weight_limit: WeightLimit,
		) -> Result<Transferred<T::AccountId>, DispatchError> {
			Self::do_transfer_assets(who, assets, fee, dest, dest_weight_limit)
		}
	}
}

/// Returns amount if `asset` is fungible, or zero.
fn fungible_amount(asset: &Asset) -> u128 {
	if let Fungible(amount) = &asset.fun {
		*amount
	} else {
		Zero::zero()
	}
}

fn half(asset: &Asset) -> Asset {
	let half_amount = fungible_amount(asset)
		.checked_div(2)
		.expect("div 2 can't overflow; qed");
	Asset {
		fun: Fungible(half_amount),
		id: asset.id.clone(),
	}
}

fn subtract_fee(asset: &Asset, amount: u128) -> Asset {
	let final_amount = fungible_amount(asset).checked_sub(amount).expect("fee too low; qed");
	Asset {
		fun: Fungible(final_amount),
		id: asset.id.clone(),
	}
}
