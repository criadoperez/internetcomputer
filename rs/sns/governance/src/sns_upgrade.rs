use crate::pb::v1::governance_error::ErrorType;
use crate::pb::v1::GovernanceError;
use crate::types::Environment;
use candid::{Decode, Encode};
use ic_base_types::{CanisterId, PrincipalId};
use ic_ic00_types::CanisterStatusResultV2;
use ic_nns_constants::SNS_WASM_CANISTER_ID;

// TODO(NNS1-1590) make these methods pub instead of pub(crate) after we no longer are duplicating
// the type definitions.  They are only that way to avoid leaking the types as they are not intended
// to be exposed beyond our workaround implementation.

/// Takes the sns_canisters along with the current version and next version and returns the canister
/// to be upgraded and the WASM it should receive.
///
/// Returns Err when more than one canister is eligible to be upgraded, or the WASM cannot be obtained
pub(crate) async fn get_upgrade_info(
    env: &dyn Environment,
    sns_canisters: &ListSnsCanistersResponse,
    current_version: &SnsVersion,
    next_version: &SnsVersion,
) -> Result<(CanisterId, Vec<u8>), GovernanceError> {
    let mut differences = vec![];
    if current_version.root_wasm_hash != next_version.root_wasm_hash {
        differences.push(SnsCanisterType::Root);
    }
    if current_version.governance_wasm_hash != next_version.governance_wasm_hash {
        differences.push(SnsCanisterType::Governance);
    }
    if current_version.ledger_wasm_hash != next_version.ledger_wasm_hash {
        differences.push(SnsCanisterType::Ledger);
    }
    if current_version.swap_wasm_hash != next_version.swap_wasm_hash {
        differences.push(SnsCanisterType::Swap);
    }

    // This should be impossible due to upstream constraints.
    if differences.is_empty() {
        return Err(GovernanceError::new_with_message(
            ErrorType::InvalidProposal,
            "No difference was found between the current SNS version and the next SNS version",
        ));
    }

    // This should also be impossible due to upstream constraints.
    if differences.len() > 1 {
        return Err(GovernanceError::new_with_message(
            ErrorType::InvalidProposal,
            "There is more than one upgrade possible for UpgradeSnsToNextVersion Action.  This is not currently supported."
        ));
    }

    let get_canister_id = |maybe_canister_principal: Option<PrincipalId>, label: &str| {
        CanisterId::new(maybe_canister_principal.unwrap_or_else(|| {
            panic!(
                "Did not receive {} CanisterId from list_sns_canisters",
                label
            )
        }))
        .unwrap()
    };

    let canister_type = differences.remove(0);

    let (canister_id, wasm_hash) = match canister_type {
        SnsCanisterType::Root => (
            get_canister_id(sns_canisters.root, "Root"),
            next_version.root_wasm_hash.clone(),
        ),
        SnsCanisterType::Governance => (
            get_canister_id(sns_canisters.governance, "Governance"),
            next_version.governance_wasm_hash.clone(),
        ),
        SnsCanisterType::Ledger => (
            get_canister_id(sns_canisters.ledger, "Ledger"),
            next_version.ledger_wasm_hash.clone(),
        ),
        SnsCanisterType::Swap => (
            get_canister_id(sns_canisters.swap, "Swap"),
            next_version.swap_wasm_hash.clone(),
        ),
        _ => {
            panic!("Cannot get here, invalid value")
        }
    };

    let response = env
        .call_canister(
            SNS_WASM_CANISTER_ID,
            "get_wasm",
            Encode!(&GetWasmRequest { hash: wasm_hash }).unwrap(),
        )
        .await
        .expect("Call to get_wasm failed");

    let response = Decode!(&response, GetWasmResponse).expect("Decoding GetWasmResponse failed");
    let wasm = response
        .wasm
        .expect("No WASM found using hash returned from SNS-WASM canister.");

    let returned_canister_type = SnsCanisterType::from_i32(wasm.canister_type).unwrap();

    if returned_canister_type != canister_type {
        panic!(
            "WASM returned from SNS-WASM is not intended for the same canister type.  \
            Expected: {:?}.  Received: {:?}.",
            canister_type,
            SnsCanisterType::from_i32(wasm.canister_type).unwrap()
        );
    }

    Ok((canister_id, wasm.wasm))
}

/// Get the current version of the SNS this SNS is using.
pub(crate) async fn get_current_version(
    env: &dyn Environment,
    root_canister_id: CanisterId,
) -> SnsVersion {
    let arg = Encode!(&GetSnsCanistersSummaryRequest {}).unwrap();

    let response = env
        .call_canister(root_canister_id, "get_sns_canisters_summary", arg)
        .await
        .expect("Request failed for get_sns_canisters_summary");

    let response = Decode!(&response, GetSnsCanistersSummaryResponse).unwrap();

    let root = response.root.unwrap();
    let governance = response.governance.unwrap();
    let swap = response.swap.unwrap();
    let ledger = response.ledger.unwrap();
    // TODO(NNS1-1576) Incorporate version into response from this method + handle errors if mismatched
    let _archives = response.archives;

    let get_hash = |canister_status: CanisterSummary, label: &str| {
        canister_status
            .status
            .unwrap_or_else(|| panic!("{} had no status", label))
            .module_hash()
            .unwrap_or_else(|| panic!("{} Status had no module hash", label))
    };

    SnsVersion {
        root_wasm_hash: get_hash(root, "Root"),
        governance_wasm_hash: get_hash(governance, "Governance"),
        ledger_wasm_hash: get_hash(ledger, "Ledger"),
        swap_wasm_hash: get_hash(swap, "Swap"),
    }
}

/// Get the next version of the SNS based on a given version.
pub(crate) async fn get_next_version(
    env: &dyn Environment,
    current_version: &SnsVersion,
) -> Option<SnsVersion> {
    let arg = Encode!(&GetNextSnsVersionRequest {
        current_version: Some(current_version.clone())
    })
    .unwrap();

    let response = env
        .call_canister(SNS_WASM_CANISTER_ID, "get_next_sns_version", arg)
        .await
        .expect("Request failed for get_next_sns_version");

    let response = Decode!(&response, GetNextSnsVersionResponse)
        .expect("Could not decode response to get_next_sns_version");

    response.next_version
}

/// Returns all SNS canisters known by the Root canister.
pub(crate) async fn get_all_sns_canisters(
    env: &dyn Environment,
    root_canister_id: CanisterId,
) -> ListSnsCanistersResponse {
    let arg = Encode!(&ListSnsCanistersRequest {}).unwrap();

    let response = env
        .call_canister(root_canister_id, "list_sns_canisters", arg)
        .await
        .expect("Did not get a valid response from root canister for list_sns_canisters request");

    return Decode!(&response, ListSnsCanistersResponse).expect("Could not decode response");
}

// TODO(NNS1-1590) Remove following duplicate definitions and split the types into their own crates

/// Duplicated from ic-sns-root to avoid circular dependency as a temporary workaround
/// See ic_sns_root::pb::v1::ListSnsCanistersRequest
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct ListSnsCanistersRequest {}

/// Duplicated from ic-sns-root to avoid circular dependency as a temporary workaround
/// See ic_sns_root::pb::v1::ListSnsCanistersRequest
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct ListSnsCanistersResponse {
    #[prost(message, optional, tag = "1")]
    pub root: ::core::option::Option<::ic_base_types::PrincipalId>,
    #[prost(message, optional, tag = "2")]
    pub governance: ::core::option::Option<::ic_base_types::PrincipalId>,
    #[prost(message, optional, tag = "3")]
    pub ledger: ::core::option::Option<::ic_base_types::PrincipalId>,
    #[prost(message, optional, tag = "4")]
    pub swap: ::core::option::Option<::ic_base_types::PrincipalId>,
    #[prost(message, repeated, tag = "5")]
    pub dapps: ::prost::alloc::vec::Vec<::ic_base_types::PrincipalId>,
    #[prost(message, repeated, tag = "6")]
    pub archives: ::prost::alloc::vec::Vec<::ic_base_types::PrincipalId>,
}

/// Duplicated from ic-sns-wasms to avoid circular dependency as a temporary workaround
/// The request type accepted by the get_next_sns_version canister method
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct GetNextSnsVersionRequest {
    #[prost(message, optional, tag = "1")]
    pub current_version: ::core::option::Option<SnsVersion>,
}

/// Duplicated from ic-sns-wasms to avoid circular dependency as a temporary workaround
/// The response type returned by the get_next_sns_version canister method
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct GetNextSnsVersionResponse {
    #[prost(message, optional, tag = "1")]
    pub next_version: ::core::option::Option<SnsVersion>,
}

/// Duplicated from ic-sns-wasms to avoid circular dependency as a temporary workaround
/// Specifies the version of an SNS
#[derive(candid::CandidType, candid::Deserialize, Eq, Hash, Clone, PartialEq, ::prost::Message)]
pub(crate) struct SnsVersion {
    /// The hash of the Root canister WASM
    #[prost(bytes = "vec", tag = "1")]
    pub root_wasm_hash: ::prost::alloc::vec::Vec<u8>,
    /// The hash of the Governance canister WASM
    #[prost(bytes = "vec", tag = "2")]
    pub governance_wasm_hash: ::prost::alloc::vec::Vec<u8>,
    /// The hash of the Ledger canister WASM
    #[prost(bytes = "vec", tag = "3")]
    pub ledger_wasm_hash: ::prost::alloc::vec::Vec<u8>,
    /// The hash of the Swap canister WASM
    #[prost(bytes = "vec", tag = "4")]
    pub swap_wasm_hash: ::prost::alloc::vec::Vec<u8>,
}

/// Copied from ic-sns-root
#[derive(PartialEq, Eq, Debug, candid::CandidType, candid::Deserialize)]
pub(crate) struct GetSnsCanistersSummaryRequest {
    // This struct intentionally left blank (for now).
}

/// Copied from ic-sns-root
#[derive(PartialEq, Eq, Debug, candid::CandidType, candid::Deserialize)]
pub(crate) struct GetSnsCanistersSummaryResponse {
    pub root: Option<CanisterSummary>,
    pub governance: Option<CanisterSummary>,
    pub ledger: Option<CanisterSummary>,
    pub swap: Option<CanisterSummary>,
    pub dapps: Vec<CanisterSummary>,
    pub archives: Vec<CanisterSummary>,
}

/// Copied from ic-sns-root
#[derive(PartialEq, Eq, Debug, candid::CandidType, candid::Deserialize)]
pub(crate) struct CanisterSummary {
    pub canister_id: Option<PrincipalId>,
    pub status: Option<CanisterStatusResultV2>,
}

///Copied from ic-sns-wasm.
/// The argument for get_wasm, which consists of the WASM hash to be retrieved.
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct GetWasmRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
}
/// Copied from ic-sns-wasm.
/// The response for get_wasm, which returns a WASM if it is found, or None.
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct GetWasmResponse {
    #[prost(message, optional, tag = "1")]
    pub wasm: ::core::option::Option<SnsWasm>,
}

/// Copied from ic-sns-wasm.
/// The representation of a WASM along with its target canister type
#[derive(candid::CandidType, candid::Deserialize, Clone, PartialEq, ::prost::Message)]
pub(crate) struct SnsWasm {
    #[prost(bytes = "vec", tag = "1")]
    pub wasm: ::prost::alloc::vec::Vec<u8>,
    #[prost(enumeration = "SnsCanisterType", tag = "2")]
    pub canister_type: i32,
}
/// Copied from ic-sns-wasm
/// The type of canister a particular WASM is intended to be installed on
#[derive(
    candid::CandidType,
    candid::Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    ::prost::Enumeration,
)]
#[repr(i32)]
pub(crate) enum SnsCanisterType {
    Unspecified = 0,
    /// The type for the root canister
    Root = 1,
    /// The type for the governance canister
    Governance = 2,
    /// The type for the ledger canister
    Ledger = 3,
    /// The type for the swap canister
    Swap = 4,
}
