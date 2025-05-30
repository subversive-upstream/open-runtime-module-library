//! Macro for benchmarking a Substrate runtime. A fork of `frame-benchmarking`
//! pallet.

#![cfg_attr(not(feature = "std"), no_std)]

mod tests;

pub use frame_benchmarking::{
	benchmarking, whitelisted_caller, BenchmarkBatch, BenchmarkConfig, BenchmarkError, BenchmarkList,
	BenchmarkMetadata, BenchmarkParameter, BenchmarkRecording, BenchmarkResult, Benchmarking, BenchmarkingSetup,
	Recording,
};

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::{format, string::String};
#[cfg(feature = "std")]
pub use frame_benchmarking::{Analysis, BenchmarkSelector};
#[doc(hidden)]
pub use frame_support;
#[doc(hidden)]
pub use log;
#[doc(hidden)]
pub use parity_scale_codec;
#[doc(hidden)]
pub use paste;
#[doc(hidden)]
pub use sp_core::defer;
#[doc(hidden)]
pub use sp_io::storage::root as storage_root;
#[doc(hidden)]
pub use sp_runtime::traits::Zero;
#[doc(hidden)]
pub use sp_std::{self, boxed::Box, prelude::Vec, str, vec};
#[doc(hidden)]
pub use sp_storage::{StateVersion, TrackedStorageKey};

/// Whitelist the given account.
#[macro_export]
macro_rules! whitelist_account {
	($acc:ident) => {
		frame_benchmarking::benchmarking::add_to_whitelist(
			frame_system::Account::<Runtime>::hashed_key_for(&$acc).into(),
		);
	};
}

/// Construct pallet benchmarks for weighing dispatchables.
///
/// Works around the idea of complexity parameters, named by a single letter
/// (which is usually upper cased in complexity notation but is lower-cased for
/// use in this macro).
///
/// Complexity parameters ("parameters") have a range which is a `u32` pair.
/// Every time a benchmark is prepared and run, this parameter takes a concrete
/// value within the range. There is an associated instancing block, which is a
/// single expression that is evaluated during preparation. It may use `?`
/// (`i.e. `return Err(...)`) to bail with a string error. Here's a
/// few examples:
///
/// ```ignore
/// // These two are equivalent:
/// let x in 0 .. 10;
/// let x in 0 .. 10 => ();
/// // This one calls a setup function and might return an error (which would be terminal).
/// let y in 0 .. 10 => setup(y)?;
/// // This one uses a code block to do lots of stuff:
/// let z in 0 .. 10 => {
///   let a = z * z / 5;
///   let b = do_something(a)?;
///   combine_into(z, b);
/// }
/// ```
///
/// Note that due to parsing restrictions, if the `from` expression is not a
/// single token (i.e. a literal or constant), then it must be parenthesised.
///
/// The macro allows for a number of "arms", each representing an individual
/// benchmark. Using the simple syntax, the associated dispatchable function
/// maps 1:1 with the benchmark and the name of the benchmark is the same as
/// that of the associated function. However, extended syntax allows
/// for arbitrary expression to be evaluated in a benchmark (including for
/// example, `on_initialize`).
///
/// Note that the ranges are *inclusive* on both sides. This is in contrast to
/// ranges in Rust which are left-inclusive right-exclusive.
///
/// Each arm may also have a block of code which is run prior to any instancing
/// and a block of code which is run afterwards. All code blocks may draw upon
/// the specific value of each parameter at any time. Local variables are shared
/// between the two pre- and post- code blocks, but do not leak from the
/// interior of any instancing expressions.
///
/// Example:
/// ```ignore
/// use path_to_node_runtime::MyRuntime;
/// use path_to_account_id::AccountId;
/// use frame_benchmarking::account;
/// use orml_benchmarking::runtime_benchmarks;
///
/// runtime_benchmarks! {
///   // The constructed runtime struct, and the pallet to benchmark.
///   { MyRuntime, my_pallet }
///
///
///   // first dispatchable: foo; this is a user dispatchable and operates on a `u8` vector of
///   // size `l`, which we allow to be initialized as usual.
///   foo {
///     let caller = account::<AccountId>(b"caller", 0, benchmarks_seed);
///     let l in 1 .. MAX_LENGTH => initialize_l(l);
///   }: _(RuntimeOrigin::signed(caller), vec![0u8; l])
///
///   // second dispatchable: bar; this is a root dispatchable and accepts a `u8` vector of size
///   // `l`.
///   // In this case, we explicitly name the call using `bar` instead of `_`.
///   bar {
///     let l in 1 .. MAX_LENGTH => initialize_l(l);
///   }: bar(RuntimeOrigin::root, vec![0u8; l])
///
///   // third dispatchable: baz; this is a user dispatchable. It isn't dependent on length like the
///   // other two but has its own complexity `c` that needs setting up. It uses `caller` (in the
///   // pre-instancing block) within the code block. This is only allowed in the param instancers
///   // of arms.
///   baz1 {
///     let caller = account::<AccountId>(b"caller", 0, benchmarks_seed);
///     let c = 0 .. 10 => setup_c(&caller, c);
///   }: baz(RuntimeOrigin::signed(caller))
///
///   // this is a second benchmark of the baz dispatchable with a different setup.
///   baz2 {
///     let caller = account::<AccountId>(b"caller", 0, benchmarks_seed);
///     let c = 0 .. 10 => setup_c_in_some_other_way(&caller, c);
///   }: baz(RuntimeOrigin::signed(caller))
///
///   // this is benchmarking some code that is not a dispatchable.
///   populate_a_set {
///     let x in 0 .. 10_000;
///     let mut m = Vec::<u32>::new();
///     for i in 0..x {
///       m.insert(i);
///     }
///   }: { m.into_iter().collect::<BTreeSet>() }
/// }
/// ```
///
/// Test functions are automatically generated for each benchmark and are
/// accessible to you when you run `cargo test`. All tests are named
/// `test_benchmark_<benchmark_name>`, expect you to pass them the Runtime
/// Config, and run them in a test externalities environment. The test function
/// runs your benchmark just like a regular benchmark, but only testing at the
/// lowest and highest values for each component. The function will return
/// `Ok(())` if the benchmarks return no errors.
///
/// You can optionally add a `verify` code block at the end of a benchmark to
/// test any final state of your benchmark in a unit test. For example:
///
/// ```ignore
/// sort_vector {
///     let x in 1 .. 10000;
///     let mut m = Vec::<u32>::new();
///     for i in (0..x).rev() {
///         m.push(i);
///     }
/// }: {
///     m.sort();
/// } verify {
///     ensure!(m[0] == 0, "You forgot to sort!")
/// }
/// ```
///
/// These `verify` blocks will not affect your benchmark results!
///
/// You can construct benchmark tests like so:
///
/// ```ignore
/// #[test]
/// fn test_benchmarks() {
///   new_test_ext().execute_with(|| {
///     assert_ok!(test_benchmark_dummy());
///     assert_err!(test_benchmark_other_name(), "Bad origin");
///     assert_ok!(test_benchmark_sort_vector());
///     assert_err!(test_benchmark_broken_benchmark(), "You forgot to sort!");
///   });
/// }
/// ```
#[macro_export]
macro_rules! runtime_benchmarks {
	(
		{ $runtime:ident, $pallet:ident }
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter!(
			{ }
			$runtime
			$pallet
			( )
			( )
			( )
			$( $rest )*
		);
	}
}

/// Same as [`benchmarks`] but for instantiable module.
#[macro_export]
macro_rules! runtime_benchmarks_instance {
	(
		{ $runtime:ident, $pallet:ident, $instance:ident }
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter!(
			{ $instance }
			$runtime
			$pallet
			( )
			( )
			( )
			$( $rest )*
		);
	}
}

#[macro_export]
#[doc(hidden)]
macro_rules! benchmarks_iter {
	// detect and extract `#[extra] tag:
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		#[extra]
		$name:ident
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter! {
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* )
			( $( $names_extra )* $name )
			( $( $names_skip_meta )* )
			$name
			$( $rest )*
		}
	};
	// mutation arm:
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* ) // This contains $( $( { $instance } )? $name:ident )*
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: _ $(< $origin_type:ty>)? ( $origin:expr $( , $arg:expr )* )
		verify $postcode:block
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter! {
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* )
			( $( $names_extra )* )
			( $( $names_skip_meta )* )
			$name { $( $code )* }: $name $(< $origin_type >)? ( $origin $( , $arg )* )
			verify $postcode
			$( $rest )*
		}
	};
	// mutation arm:
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: $dispatch:ident $(< $origin_type:ty>)? ( $origin:expr $( , $arg:expr )* )
		verify $postcode:block
		$( $rest:tt )*
	) => {
		$crate::paste::paste! {
			$crate::benchmarks_iter! {
				{ $( $instance)? }
				$runtime
				$pallet
				( $( $names )* )
					( $( $names_extra )* )
					( $( $names_skip_meta )* )
					$name {
						$( $code )*
							let __call =
							$pallet::Call::<$runtime $(, $instance )?
									>:: [< new_call_variant_ $dispatch >] (
								$($arg),*
							);
						let __benchmarked_call_encoded = $crate::parity_scale_codec::Encode::encode(
							&__call
						);
					}: {
						let __call_decoded = <
								$pallet::Call::<$runtime $(, $instance )?>
								as $crate::parity_scale_codec::Decode
								>::decode(&mut &__benchmarked_call_encoded[..])
							.expect("call is encoded above, encoding must be correct");
						let __origin = $crate::to_origin!($origin $(, $origin_type)?);
						<$pallet::Call::<$runtime $(, $instance )?> as $crate::frame_support::traits::UnfilteredDispatchable
							>::dispatch_bypass_filter(__call_decoded, __origin)?;
					}
				verify $postcode
					$( $rest )*
			}
		}
	};
	// iteration arm:
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: $eval:block
		verify $postcode:block
		$( $rest:tt )*
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{ }
			{ $eval }
			{ $( $code )* }
			$postcode
		}

		#[cfg(test)]
		$crate::impl_benchmark_test!(
			$runtime
			$pallet
			{ $( $instance)? }
			$name
		);

		$crate::benchmarks_iter!(
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* { $( $instance )? } $name )
			( $( $names_extra )* )
			( $( $names_skip_meta )* )
			$( $rest )*
		);
	};
	// iteration-exit arm
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
	) => {
		$crate::selected_benchmark!(
			$runtime
			$pallet
			{ $( $instance)? }
			$( $names )*
		);
		$crate::impl_benchmark!(
			$runtime
			$pallet
			{ $( $instance)? }
			( $( $names )* )
			( $( $names_extra ),* )
			( $( $names_skip_meta )* )
		);
	};
	// add verify block to _() format
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: _ ( $origin:expr $( , $arg:expr )* )
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter! {
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* )
			( $( $names_extra )* )
			( $( $names_skip_meta )* )
			$name { $( $code )* }: _ ( $origin $( , $arg )* )
			verify { }
			$( $rest )*
		}
	};
	// add verify block to name() format
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: $dispatch:ident ( $origin:expr $( , $arg:expr )* )
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter! {
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* )
			( $( $names_extra )* )
			( $( $names_skip_meta )* )
			$name { $( $code )* }: $dispatch ( $origin $( , $arg )* )
			verify { }
			$( $rest )*
		}
	};
	// add verify block to {} format
	(
		{ $( $instance:ident )? }
		$runtime:ident
		$pallet:ident
		( $( $names:tt )* )
		( $( $names_extra:tt )* )
		( $( $names_skip_meta:tt )* )
		$name:ident { $( $code:tt )* }: $eval:block
		$( $rest:tt )*
	) => {
		$crate::benchmarks_iter!(
			{ $( $instance)? }
			$runtime
			$pallet
			( $( $names )* )
			( $( $names_extra )* )
			( $( $names_skip_meta )* )
			$name { $( $code )* }: $eval
			verify { }
			$( $rest )*
		);
	};
}

#[macro_export]
#[doc(hidden)]
macro_rules! to_origin {
	($origin:expr) => {
		$origin.into()
	};
	($origin:expr, $origin_type:ty) => {
		<<$runtime as frame_system::Config>::RuntimeOrigin as From<$origin_type>>::from($origin)
	};
}

#[macro_export]
#[doc(hidden)]
macro_rules! benchmark_backend {
	// parsing arms
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{ $( PRE { $( $pre_parsed:tt )* } )* }
		{ $eval:block }
		{
			let $pre_id:tt : $pre_ty:ty = $pre_ex:expr;
			$( $rest:tt )*
		}
		$postcode:block
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{
				$( PRE { $( $pre_parsed )* } )*
				PRE { $pre_id , $pre_ty , $pre_ex }
			}
			{ $eval }
			{ $( $rest )* }
			$postcode
		}
	};
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{ $( $parsed:tt )* }
		{ $eval:block }
		{
			let $param:ident in ( $param_from:expr ) .. $param_to:expr => $param_instancer:expr;
			$( $rest:tt )*
		}
		$postcode:block
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{
				$( $parsed )*
				PARAM { $param , $param_from , $param_to , $param_instancer }
			}
			{ $eval }
			{ $( $rest )* }
			$postcode
		}
	};
	// mutation arm to look after a single tt for param_from.
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{ $( $parsed:tt )* }
		{ $eval:block }
		{
			let $param:ident in $param_from:tt .. $param_to:expr => $param_instancer:expr ;
			$( $rest:tt )*
		}
		$postcode:block
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{ $( $parsed )* }
			{ $eval }
			{
				let $param in ( $param_from ) .. $param_to => $param_instancer;
				$( $rest )*
			}
			$postcode
		}
	};
	// mutation arm to look after the default tail of `=> ()`
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{ $( $parsed:tt )* }
		{ $eval:block }
		{
			let $param:ident in $param_from:tt .. $param_to:expr;
			$( $rest:tt )*
		}
		$postcode:block
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{ $( $parsed )* }
			{ $eval }
			{
				let $param in $param_from .. $param_to => ();
				$( $rest )*
			}
			$postcode
		}
	};
	// mutation arm to look after `let _ =`
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{ $( $parsed:tt )* }
		{ $eval:block }
		{
			let $pre_id:tt = $pre_ex:expr;
			$( $rest:tt )*
		}
		$postcode:block
	) => {
		$crate::benchmark_backend! {
			{ $( $instance)? }
			$name
			$runtime
			$pallet
			{ $( $parsed )* }
			{ $eval }
			{
				let $pre_id : _ = $pre_ex;
				$( $rest )*
			}
			$postcode
		}
	};
	// actioning arm
	(
		{ $( $instance:ident )? }
		$name:ident
		$runtime:ident
		$pallet:ident
		{
			$( PRE { $pre_id:tt , $pre_ty:ty , $pre_ex:expr } )*
			$( PARAM { $param:ident , $param_from:expr , $param_to:expr , $param_instancer:expr } )*
		}
		{ $eval:block }
		{ $( $post:tt )* }
		$postcode:block
	) => {
		#[allow(non_camel_case_types)]
		struct $name;
		#[allow(unused_variables)]
		impl $crate::BenchmarkingSetup<$runtime $(, $instance)?> for $name {
			fn components(&self) -> $crate::Vec<($crate::BenchmarkParameter, u32, u32)> {
				$crate::vec! [
					$(
						($crate::BenchmarkParameter::$param, $param_from, $param_to)
					),*
				]
			}

			fn instance(
				&self,
				recording: &mut impl $crate::Recording,
				components: &[($crate::BenchmarkParameter, u32)],
				verify: bool
			) -> Result<(), $crate::BenchmarkError> {
				$(
					// Prepare instance
					let $param = components.iter()
						.find(|&c| c.0 == $crate::BenchmarkParameter::$param)
						.ok_or("Could not find component in benchmark preparation.")?
						.1;
				)*
				$(
					let $pre_id : $pre_ty = $pre_ex;
				)*
				$( $param_instancer ; )*
				$( $post )*

				recording.start();
                $eval;
                recording.stop();

				if verify {
					$postcode;
				}
				Ok(())
			}
		}
	};
}

// Creates a `SelectedBenchmark` enum implementing `BenchmarkingSetup`.
//
// Every variant must implement [`BenchmarkingSetup`].
//
// ```nocompile
// struct Transfer;
// impl BenchmarkingSetup for Transfer { ... }
//
// struct SetBalance;
// impl BenchmarkingSetup for SetBalance { ... }
//
// selected_benchmark!({} Transfer {} SetBalance);
// ```
#[macro_export]
#[doc(hidden)]
macro_rules! selected_benchmark {
	(
		$runtime:ident
		$pallet:ident
		{ $( $instance:ident )? }
		$( { $( $bench_inst:ident )? } $bench:ident )*
	) => {
		// The list of available benchmarks for this pallet.
		#[allow(non_camel_case_types)]
		enum SelectedBenchmark {
			$( $bench, )*
		}

		// Allow us to select a benchmark from the list of available benchmarks.
		impl $crate::BenchmarkingSetup<$runtime $(, $instance)?> for SelectedBenchmark {
			fn components(&self) -> $crate::Vec<($crate::BenchmarkParameter, u32, u32)> {
				match self {
					$(
						Self::$bench => <
							$bench as $crate::BenchmarkingSetup<$runtime $(, $bench_inst)? >
						>::components(&$bench),
					)*
				}
			}

			fn instance(
				&self,
				recording: &mut impl $crate::Recording,
				components: &[($crate::BenchmarkParameter, u32)],
				verify: bool
			) -> Result<(), $crate::BenchmarkError> {
				match self {
					$(
						Self::$bench => <
							$bench as $crate::BenchmarkingSetup<$runtime $(, $bench_inst)? >
						>::instance(&$bench, recording, components, verify),
					)*
				}
			}
		}
	};
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_benchmark {
	(
		$runtime:ident
		$pallet:ident
		{ $( $instance:ident )? }
		( $( { $( $name_inst:ident )? } $name:ident )* )
		( $( $name_extra:ident ),* )
		( $( $name_skip_meta:ident ),* )
	) => {
		pub struct Benchmark;

		impl $crate::Benchmarking for Benchmark {
			fn benchmarks(extra: bool) -> $crate::Vec<$crate::BenchmarkMetadata> {
				let mut all_names = $crate::vec![ $( stringify!($name).as_ref() ),* ];
				if !extra {
					let extra = [ $( stringify!($name_extra).as_ref() ),* ];
					all_names.retain(|x| !extra.contains(x));
				}
				all_names.into_iter().map(|benchmark| {
					let selected_benchmark = match benchmark {
						$( stringify!($name) => SelectedBenchmark::$name, )*
						_ => panic!("all benchmarks should be selectable"),
					};
					let components = <
						SelectedBenchmark as $crate::BenchmarkingSetup<$runtime $(, $instance)?>
					>::components(&selected_benchmark);

					$crate::BenchmarkMetadata {
						name: benchmark.as_bytes().to_vec(),
						components,
						// TODO: Not supported by V2 syntax as of yet.
						// https://github.com/paritytech/substrate/issues/13132
						pov_modes: vec![],
					}
				}).collect::<$crate::Vec<_>>()
			}

			fn run_benchmark(
				extrinsic: &[u8],
				c: &[($crate::BenchmarkParameter, u32)],
				whitelist: &[$crate::TrackedStorageKey],
				verify: bool,
				internal_repeats: u32,
			) -> Result<$crate::Vec<$crate::BenchmarkResult>, $crate::BenchmarkError> {
				// Map the input to the selected benchmark.
				let extrinsic = $crate::str::from_utf8(extrinsic)
					.map_err(|_| "`extrinsic` is not a valid utf8 string!")?;
				let selected_benchmark = match extrinsic {
					$( stringify!($name) => SelectedBenchmark::$name, )*
					_ => return Err("Could not find extrinsic.".into()),
				};

				// Add whitelist to DB including whitelisted caller
				let mut whitelist = whitelist.to_vec();
				let whitelisted_caller_key =
					<frame_system::Account::<$runtime> as $crate::frame_support::storage::StorageMap<_,_>>::hashed_key_for(
						$crate::whitelisted_caller::<<$runtime as frame_system::Config>::AccountId>()
					);
				whitelist.push(whitelisted_caller_key.into());
				// Whitelist the transactional layer.
				let transactional_layer_key = $crate::TrackedStorageKey::new(
					$crate::frame_support::storage::transactional::TRANSACTION_LEVEL_KEY.into()
				);
				whitelist.push(transactional_layer_key);
				$crate::benchmarking::set_whitelist(whitelist);

				let mut results: $crate::Vec<$crate::BenchmarkResult> = $crate::Vec::new();

				let on_before_start = || {
					// Set the block number to at least 1 so events are deposited.
					if $crate::Zero::is_zero(&frame_system::Pallet::<$runtime>::block_number()) {
						frame_system::Pallet::<$runtime>::set_block_number(1u32.into());
					}

					// Commit the externalities to the database, flushing the DB cache.
					// This will enable worst case scenario for reading from the database.
					$crate::benchmarking::commit_db();

					// Reset the read/write counter so we don't count operations in the setup process.
					$crate::benchmarking::reset_read_write_count();
				};

				// Always do at least one internal repeat...
				for _ in 0 .. internal_repeats.max(1) {
					// Always reset the state after the benchmark.
					$crate::defer!($crate::benchmarking::wipe_db());

					// Time the extrinsic logic.
					$crate::log::trace!(
						target: "benchmark",
						"Start Benchmark: {:?}", c
					);

					let mut recording = $crate::BenchmarkRecording::new(&on_before_start);
					<SelectedBenchmark as $crate::BenchmarkingSetup<$runtime>>::instance(&selected_benchmark, &mut recording, c, verify)?;

					// Calculate the diff caused by the benchmark.
					let elapsed_extrinsic = recording.elapsed_extrinsic().expect("elapsed time should be recorded");
					let diff_pov = recording.diff_pov().unwrap_or_default();

					// Commit the changes to get proper write count
					$crate::benchmarking::commit_db();
					$crate::log::trace!(
						target: "benchmark",
						"End Benchmark: {} ns", elapsed_extrinsic
					);
					let read_write_count = $crate::benchmarking::read_write_count();
					$crate::log::trace!(
						target: "benchmark",
						"Read/Write Count {:?}", read_write_count
					);

					// Time the storage root recalculation.
					let start_storage_root = $crate::benchmarking::current_time();
					$crate::storage_root($crate::StateVersion::V0);
					let finish_storage_root = $crate::benchmarking::current_time();
					let elapsed_storage_root = finish_storage_root - start_storage_root;

					let skip_meta = [ $( stringify!($name_skip_meta).as_ref() ),* ];
					let read_and_written_keys = if skip_meta.contains(&extrinsic) {
						$crate::vec![(b"Skipped Metadata".to_vec(), 0, 0, false)]
					} else {
						$crate::benchmarking::get_read_and_written_keys()
					};

					results.push($crate::BenchmarkResult {
						components: c.to_vec(),
						extrinsic_time: elapsed_extrinsic,
						storage_root_time: elapsed_storage_root,
						reads: read_write_count.0,
						repeat_reads: read_write_count.1,
						writes: read_write_count.2,
						repeat_writes: read_write_count.3,
						proof_size: diff_pov,
						keys: read_and_written_keys,
					});

					// Wipe the DB back to the genesis state.
					$crate::benchmarking::wipe_db();
				}

				return Ok(results);
			}
		}

		#[cfg(test)]
		impl Benchmark {
			/// Test a particular benchmark by name.
			///
			/// This isn't called `test_benchmark_by_name` just in case some end-user eventually
			/// writes a benchmark, itself called `by_name`; the function would be shadowed in
			/// that case.
			///
			/// This is generally intended to be used by child test modules such as those created
			/// by the `impl_benchmark_test_suite` macro. However, it is not an error if a pallet
			/// author chooses not to implement benchmarks.
			#[allow(unused)]
			fn test_bench_by_name(name: &[u8]) -> Result<(), $crate::BenchmarkError> {
				let name = $crate::str::from_utf8(name)
					.map_err(|_| -> $crate::BenchmarkError { "`name` is not a valid utf8 string!".into() })?;
				match name {
					$( stringify!($name) => {
						$crate::paste::paste! { Self::[< test_benchmark_ $name >]() }
					} )*
					_ => Err("Could not find test for requested benchmark.".into()),
				}
			}
		}
	};
}

// This creates a unit test for one benchmark of the main benchmark macro.
// It runs the benchmark using the `high` and `low` value for each component
// and ensure that everything completes successfully.
#[macro_export]
#[doc(hidden)]
macro_rules! impl_benchmark_test {
	(
		$runtime:ident
		$pallet:ident
		{ $( $instance:ident )? }
		$name:ident
	) => {
		$crate::paste::item! {
			impl Benchmark {
				#[allow(unused)]
				fn [<test_benchmark_ $name>] () -> Result<(), $crate::BenchmarkError> {
					let selected_benchmark = SelectedBenchmark::$name;
					let components = <
						SelectedBenchmark as $crate::BenchmarkingSetup<$runtime, _>
					>::components(&selected_benchmark);

					let execute_benchmark = |
						c: $crate::Vec<($crate::BenchmarkParameter, u32)>
					| -> Result<(), $crate::BenchmarkError> {

						let on_before_start = || {
							// Set the block number to at least 1 so events are deposited.
							if $crate::Zero::is_zero(&frame_system::Pallet::<$runtime>::block_number()) {
								frame_system::Pallet::<$runtime>::set_block_number(1u32.into());
							}
						};

						// Run execution + verification
						<SelectedBenchmark as $crate::BenchmarkingSetup<$runtime, _>>::test_instance(&selected_benchmark, &c, &on_before_start)?;

						// Reset the state
						$crate::benchmarking::wipe_db();

						Ok(())
					};

					if components.is_empty() {
						execute_benchmark(Default::default())?;
					} else {
						for (name, low, high) in components.iter() {
							// Test only the low and high value, assuming values in the middle
							// won't break
							for component_value in $crate::vec![low, high] {
								// Select the max value for all the other components.
								let c: $crate::Vec<($crate::BenchmarkParameter, u32)> = components
									.iter()
									.map(|(n, _, h)|
										if n == name {
											(*n, *component_value)
										} else {
											(*n, *h)
										}
									)
									.collect();

								execute_benchmark(c)?;
							}
						}
					}
					Ok(())
				}
			}
		}
	};
}

/// This creates a test suite which runs the module's benchmarks.
///
/// When called in `pallet_example` as
///
/// ```rust,ignore
/// impl_benchmark_test_suite!(crate::tests::new_test_ext());
/// ```
///
/// It expands to the equivalent of:
///
/// ```rust,ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     use crate::tests::new_test_ext;
///     use frame_support::assert_ok;
///
///     #[test]
///     fn test_benchmarks() {
///         new_test_ext().execute_with(|| {
///             assert_ok!(test_benchmark_accumulate_dummy());
///             assert_ok!(test_benchmark_set_dummy());
///             assert_ok!(test_benchmark_another_set_dummy());
///             assert_ok!(test_benchmark_sort_vector());
///         });
///     }
/// }
/// ```
///
/// ## Arguments
///
/// The first argument, `new_test_ext`, must be a function call which returns
/// either a `sp_io::TestExternalities`, or some other type with a similar
/// interface.
///
/// Note that this function call is _not_ evaluated at compile time, but is
/// instead copied textually into each appropriate invocation site.
///
/// There is an optional second argument, with keyword syntax: `benchmarks_path
/// = path_to_benchmarks_invocation`. In the typical case in which this macro is
/// in the same module as the `benchmarks!` invocation, you don't need to supply
/// this. However, if the `impl_benchmark_test_suite!` invocation is in a
/// different module than the `runtime_benchmarks!` invocation, then you should
/// provide the path to the module containing the `benchmarks!` invocation:
///
/// ```rust,ignore
/// mod benches {
///     runtime_benchmarks!{
///         ...
///     }
/// }
///
/// mod tests {
///     // because of macro syntax limitations, benches can't be paths, but has
///     // to be idents in the scope of `impl_benchmark_test_suite`.
///     use crate::benches;
///
///     impl_benchmark_test_suite!(new_test_ext(), benchmarks_path = benches);
///
///     // new_test_ext are defined later in this module
/// }
/// ```
///
/// There is an optional 3rd argument, with keyword syntax: `extra = true` or
/// `extra = false`. By default, this generates a test suite which iterates over
/// all benchmarks, including those marked with the `#[extra]` annotation.
/// Setting `extra = false` excludes those.
///
/// There is an optional 4th argument, with keyword syntax: `exec_name =
/// custom_exec_name`. By default, this macro uses `execute_with` for this
/// parameter. This argument, if set, is subject to these restrictions:
///
/// - It must be the name of a method applied to the output of the
///   `new_test_ext` argument.
/// - That method must have a signature capable of receiving a single argument
///   of the form `impl FnOnce()`.
#[macro_export]
macro_rules! impl_benchmark_test_suite {
	// user might or might not have set some keyword arguments; set the defaults
	//
	// The weird syntax indicates that `rest` comes only after a comma, which is otherwise optional
	(
		$new_test_ext:expr,
		$(, $( $rest:tt )* )?
	) => {
		$crate::impl_benchmark_test_suite!(
			@selected:
				$new_test_ext,
				benchmarks_path = super,
				extra = true,
				exec_name = execute_with,
			@user:
				$( $( $rest )* )?
		);
	};
	// pick off the benchmarks_path keyword argument
	(
		@selected:
			$new_test_ext:expr,
			benchmarks_path = $old:ident,
			extra = $extra:expr,
			exec_name = $exec_name:ident,
		@user:
			benchmarks_path = $benchmarks_path:ident
			$(, $( $rest:tt )* )?
	) => {
		$crate::impl_benchmark_test_suite!(
			@selected:
				$new_test_ext,
				benchmarks_path = $benchmarks_path,
				extra = $extra,
				exec_name = $exec_name,
			@user:
				$( $( $rest )* )?
		);
	};
	// pick off the extra keyword argument
	(
		@selected:
			$new_test_ext:expr,
			benchmarks_path = $benchmarks_path:ident,
			extra = $old:expr,
			exec_name = $exec_name:ident,
		@user:
			extra = $extra:expr
			$(, $( $rest:tt )* )?
	) => {
		$crate::impl_benchmark_test_suite!(
			@selected:
				$new_test_ext,
				benchmarks_path = $benchmarks_path,
				extra = $extra,
				exec_name = $exec_name,
			@user:
				$( $( $rest )* )?
		);
	};
	// pick off the exec_name keyword argument
	(
		@selected:
			$new_test_ext:expr,
			benchmarks_path = $benchmarks_path:ident,
			extra = $extra:expr,
			exec_name = $old:ident,
		@user:
			exec_name = $exec_name:ident
			$(, $( $rest:tt )* )?
	) => {
		$crate::impl_benchmark_test_suite!(
			@selected:
				$new_test_ext,
				benchmarks_path = $benchmarks_path,
				extra = $extra,
				exec_name = $exec_name,
			@user:
				$( $( $rest )* )?
		);
	};
	// all options set; nothing else in user-provided keyword arguments
	(
		@selected:
			$new_test_ext:expr,
			benchmarks_path = $path_to_benchmarks_invocation:ident,
			extra = $extra:expr,
			exec_name = $exec_name:ident,
		@user:
			$(,)?
	) => {
		#[cfg(test)]
		mod benchmark_tests {
			use super::*;

			#[test]
			fn test_benchmarks() {
				$new_test_ext.$exec_name(|| {
					use $crate::Benchmarking;

					let mut anything_failed = false;
					println!("failing benchmark tests:");
					for benchmark_metadata in $path_to_benchmarks_invocation::Benchmark::benchmarks($extra) {
						let benchmark_name = &benchmark_metadata.name;
						match std::panic::catch_unwind(|| {
							Benchmark::test_bench_by_name(benchmark_name)
						}) {
							Err(err) => {
								println!(
									"{}: {:?}",
									$crate::str::from_utf8(benchmark_name)
										.expect("benchmark name is always a valid string!"),
									err,
								);
								anything_failed = true;
							},
							Ok(Err(err)) => {
								match err {
									$crate::BenchmarkError::Stop(err) => {
										println!(
											"{}: {:?}",
											$crate::str::from_utf8(benchmark_name)
												.expect("benchmark name is always a valid string!"),
											err,
										);
										anything_failed = true;
									},
									$crate::BenchmarkError::Override(_) => {
										// This is still considered a success condition.
										$crate::log::error!(
											"WARNING: benchmark error overridden - {}",
											$crate::str::from_utf8(benchmark_name)
												.expect("benchmark name is always a valid string!"),
										);
									},
									$crate::BenchmarkError::Skip => {
										// This is considered a success condition.
										$crate::log::error!(
											"WARNING: benchmark error skipped - {}",
											$crate::str::from_utf8(benchmark_name)
												.expect("benchmark name is always a valid string!"),
										);
									},
									$crate::BenchmarkError::Weightless => {
										// This is considered a success condition.
										$crate::log::error!(
											"WARNING: benchmark error weightless skipped - {}",
											$crate::str::from_utf8(benchmark_name)
												.expect("benchmark name is always a valid string!"),
										);
									}
								}
							},
							Ok(Ok(())) => (),
						}
					}
					assert!(!anything_failed);
				});
			}
		}
	};
}

/// show error message and debugging info for the case of an error happening
/// during a benchmark
pub fn show_benchmark_debug_info(
	instance_string: &[u8],
	benchmark: &[u8],
	components: &[(BenchmarkParameter, u32)],
	verify: &bool,
	error_message: &str,
) -> String {
	format!(
		"\n* Pallet: {}\n\
		* Benchmark: {}\n\
		* Components: {:?}\n\
		* Verify: {:?}\n\
		* Error message: {}",
		sp_std::str::from_utf8(instance_string).expect("it's all just strings ran through the wasm interface. qed"),
		sp_std::str::from_utf8(benchmark).expect("it's all just strings ran through the wasm interface. qed"),
		components,
		verify,
		error_message,
	)
}

/// This macro adds pallet benchmarks to a `Vec<BenchmarkBatch>` object.
///
/// First create an object that holds in the input parameters for the benchmark:
///
/// ```ignore
/// let params = (&config, &whitelist);
/// ```
///
/// The `whitelist` is a parameter you pass to control the DB read/write
/// tracking. We use a vector of
/// [TrackedStorageKey](./struct.TrackedStorageKey.html), which is a simple
/// struct used to set if a key has been read or written to.
///
/// For values that should be skipped entirely, we can just pass `key.into()`.
/// For example:
///
/// ```
/// use sp_storage::TrackedStorageKey;
/// let whitelist: Vec<TrackedStorageKey> = vec![
///     // Block Number
///     hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef702a5c1b19ab7a04f536c519aca4983ac").to_vec().into(),
///     // Total Issuance
///     hex_literal::hex!("c2261276cc9d1f8598ea4b6a74b15c2f57c875e4cff74148e4628f264b974c80").to_vec().into(),
///     // Execution Phase
///     hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef7ff553b5a9862a516939d82b3d3d8661a").to_vec().into(),
///     // Event Count
///     hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef70a98fdbe9ce6c55837576c60c7af3850").to_vec().into(),
/// ];
/// ```
///
/// Then define a mutable local variable to hold your `BenchmarkBatch` object:
/// ```ignore
/// let mut batches = Vec::<BenchmarkBatch>::new();
/// ````
///
/// Then add the pallets you want to benchmark to this object, using their crate
/// name and generated module struct:
/// ```ignore
/// add_benchmark!(params, batches, pallet_balances, Balances);
/// add_benchmark!(params, batches, pallet_session, SessionBench::<Runtime>);
/// add_benchmark!(params, batches, frame_system, SystemBench::<Runtime>);
/// ...
/// ```
///
/// At the end of `dispatch_benchmark`, you should return this batches object.
#[macro_export]
macro_rules! add_benchmark {
	( $params:ident, $batches:ident, $name:path, $( $location:tt )* ) => (
		let name_string = stringify!($name).as_bytes();
		let instance_string = stringify!( $( $location )* ).as_bytes();
		let (config, whitelist) = $params;
		let $crate::BenchmarkConfig {
			pallet,
			benchmark,
			selected_components,
			verify,
			internal_repeats,
		} = config;
		if &pallet[..] == &name_string[..] {
			let benchmark_result = $( $location )*::Benchmark::run_benchmark(
				&benchmark[..],
				&selected_components[..],
				whitelist,
				*verify,
				*internal_repeats,
			);

			let final_results = match benchmark_result {
				Ok(results) => Some(results),
				Err($crate::BenchmarkError::Override(mut result)) => {
					// Insert override warning as the first storage key.
					$crate::log::error!(
						"WARNING: benchmark error overridden - {}",
						$crate::str::from_utf8(benchmark)
							.expect("benchmark name is always a valid string!")
					);
					result.keys.insert(0,
						(b"Benchmark Override".to_vec(), 0, 0, false)
					);
					Some($crate::vec![result])
				},
				Err($crate::BenchmarkError::Stop(e)) => {
					$crate::show_benchmark_debug_info(
						instance_string,
						benchmark,
						selected_components,
						verify,
						e,
					);
					return Err(e.into());
				},
				Err($crate::BenchmarkError::Skip) => {
					$crate::log::error!(
						"WARNING: benchmark error skipped - {}",
						$crate::str::from_utf8(benchmark)
							.expect("benchmark name is always a valid string!")
					);
					None
				},
				Err($crate::BenchmarkError::Weightless) => {
					$crate::log::error!(
						"WARNING: benchmark weightless skipped - {}",
						$crate::str::from_utf8(benchmark)
							.expect("benchmark name is always a valid string!")
					);
					Some(vec![$crate::BenchmarkResult {
						components: selected_components.clone(),
						.. Default::default()
					}])
				}
			};

			if let Some(final_results) = final_results {
				$batches.push($crate::BenchmarkBatch {
					pallet: name_string.to_vec(),
					instance: instance_string.to_vec(),
					benchmark: benchmark.clone(),
					results: final_results,
				});
			}
		}
	)
}

/// Callback for `define_benchmarks` to call `add_benchmark`.
#[macro_export]
macro_rules! cb_add_benchmarks {
    // anchor
	( $params:ident, $batches:ident, [ $name:path, $( $location:tt )* ] ) => {
        add_benchmark!( $params, $batches, $name, $( $location )* );
	};
    // recursion tail
	( $params:ident, $batches:ident, [ $name:path, $( $location:tt )* ] $([ $names:path, $( $locations:tt )* ])+ ) => {
        cb_add_benchmarks!( $params, $batches, [ $name, $( $location )* ] );
        cb_add_benchmarks!( $params, $batches, $([ $names, $( $locations )* ])+ );
    }
}

/// This macro allows users to easily generate a list of benchmarks for the
/// pallets configured in the runtime.
///
/// To use this macro, first create a an object to store the list:
///
/// ```ignore
/// let mut list = Vec::<BenchmarkList>::new();
/// ```
///
/// Then pass this `list` to the macro, along with the `extra` boolean, the
/// pallet crate, and pallet struct:
///
/// ```ignore
/// list_benchmark!(list, extra, pallet_balances, Balances);
/// list_benchmark!(list, extra, pallet_session, SessionBench::<Runtime>);
/// list_benchmark!(list, extra, frame_system, SystemBench::<Runtime>);
/// ```
///
/// This should match what exists with the `add_benchmark!` macro.
#[macro_export]
macro_rules! list_benchmark {
	( $list:ident, $extra:ident, $name:path, $( $location:tt )* ) => (
		let pallet_string = stringify!($name).as_bytes();
		let instance_string = stringify!( $( $location )* ).as_bytes();
		let benchmarks = $( $location )*::Benchmark::benchmarks($extra);
		let pallet_benchmarks = $crate::BenchmarkList {
			pallet: pallet_string.to_vec(),
			instance: instance_string.to_vec(),
			benchmarks: benchmarks.to_vec(),
		};
		$list.push(pallet_benchmarks)
	)
}

/// Callback for `define_benchmarks` to call `list_benchmark`.
#[macro_export]
macro_rules! cb_list_benchmarks {
    // anchor
	( $list:ident, $extra:ident, [ $name:path, $( $location:tt )* ] ) => {
        list_benchmark!( $list, $extra, $name, $( $location )* );
	};
    // recursion tail
	( $list:ident, $extra:ident, [ $name:path, $( $location:tt )* ] $([ $names:path, $( $locations:tt )* ])+ ) => {
        cb_list_benchmarks!( $list, $extra, [ $name, $( $location )* ] );
        cb_list_benchmarks!( $list, $extra, $([ $names, $( $locations )* ])+ );
    }
}

/// Defines pallet configs that `add_benchmarks` and `list_benchmarks` use.
/// Should be preferred instead of having a repetitive list of configs
/// in `add_benchmark` and `list_benchmark`.
#[macro_export]
macro_rules! define_benchmarks {
    ( $([ $names:path, $( $locations:tt )* ])* ) => {
		/// Calls `list_benchmark` with all configs from `define_benchmarks`
		/// and passes the first two parameters on.
		///
		/// Use as:
		/// ```ignore
		/// list_benchmarks!(list, extra);
		/// ```
		#[macro_export]
		macro_rules! list_benchmarks {
            ( $list:ident, $extra:ident ) => {
                cb_list_benchmarks!( $list, $extra, $([ $names, $( $locations )* ])+ );
            }
        }

		/// Calls `add_benchmark` with all configs from `define_benchmarks`
		/// and passes the first two parameters on.
		///
		/// Use as:
		/// ```ignore
		/// add_benchmarks!(params, batches);
		/// ```
		#[macro_export]
		macro_rules! add_benchmarks {
            ( $params:ident, $batches:ident ) => {
                cb_add_benchmarks!( $params, $batches, $([ $names, $( $locations )* ])+ );
            }
        }
    }
}
