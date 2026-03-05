#![allow(deprecated)]

use cosmwasm_std::testing::{
    mock_dependencies, mock_env, mock_info, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, ContractResult, CosmosMsg, Empty, Env, OwnedDeps,
    QuerierResult, Storage, SystemError, SystemResult, Timestamp, WasmMsg, WasmQuery,
};
use cw2::set_contract_version;
use cw721::msg::{Cw721ExecuteMsg, Cw721QueryMsg, OwnerOfResponse};
use cw721::receiver::Cw721ReceiveMsg;

use crate::contract::{execute, instantiate, migrate, query};
use crate::error::ContractError;
use crate::msg::*;
use crate::state::{AuctionStatus, AUCTIONS};

// ═══════════════════════════════════════════════════════════════════════
// TEST HELPERS
// ═══════════════════════════════════════════════════════════════════════

const ADMIN: &str = "cosmwasm1335hded4gyzpt00fpz75mms4m7ck02wgw07yhw9grahj4dzg4yvqysvwql";
const BIDDER1: &str = "cosmwasm1ypwq4qs278keswt2xhlwxhg99ntevk3h04f05w84528yvjq05yaqhtcy3x";
const BIDDER2: &str = "cosmwasm1p2hhywks3stcsthk0gznrv6xm53txcs76yxyndzapenlqpq98xqqlqwjd3";
const BIDDER3: &str = "cosmwasm1p6vhhx6ua7qjl33lk69jws524da930z7wtxsm354dttx6jkuym3szv7lm6";
const MAD_COLLECTION: &str = "cosmwasm1um0dcwqv0vf2anhva7pgraye9l6g8s4zf9y94gqd2e9scnludwmqexwpsw";
const MEGA_COLLECTION: &str = "cosmwasm1xf7mf3kfl3sq3py0ey92lwfp3d6nn5v082kmejk6g7adxr6frsnsxmnl3k";
const CONTRACT: &str = "cosmwasm1ejpjr43ht3y56pplm5pxpusmcrk9rkkvna4tklusnnwdxpqm0zls40599z";
const CREATOR: &str = "cosmwasm1h34lmpywh4upnjdg90cjf4j70aee6z8qqfspugamjp42e4q28kqs8s7vcp";

fn mock_deps_with_nft_querier() -> OwnedDeps<MockStorage, MockApi, MockQuerier, Empty> {
    let mut deps = OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: MockQuerier::new(&[]),
        custom_query_type: std::marker::PhantomData,
    };

    deps.querier.update_wasm(move |query| -> QuerierResult {
        match query {
            WasmQuery::Smart { msg, .. } => {
                let query_msg: Result<Cw721QueryMsg<Empty, Empty, Empty>, _> = from_json(msg);
                match query_msg {
                    Ok(Cw721QueryMsg::OwnerOf { .. }) => {
                        let resp = OwnerOfResponse {
                            owner: CONTRACT.to_string(),
                            approvals: vec![],
                        };
                        SystemResult::Ok(ContractResult::Ok(to_json_binary(&resp).unwrap()))
                    }
                    _ => SystemResult::Err(SystemError::UnsupportedRequest {
                        kind: "unhandled".to_string(),
                    }),
                }
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "unhandled".to_string(),
            }),
        }
    });

    deps
}

fn mock_env_at(seconds: u64) -> Env {
    let mut env = mock_env();
    env.block.time = Timestamp::from_seconds(seconds);
    env.contract.address = Addr::unchecked(CONTRACT);
    env
}

fn setup_contract(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>) {
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let info = mock_info(ADMIN, &[]);
    instantiate(deps.as_mut(), mock_env_at(1000), info, msg).unwrap();
}

/// Simulate sending a Cosmic NFT via CW721 Send -> ReceiveNft to create an auction
fn send_mega_for_auction(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    mega_token_id: &str,
    start: u64,
    end: u64,
    min_bid: Option<u64>,
) {
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: mega_token_id.to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: start,
            end_time: end,
            min_bid,
        })
        .unwrap(),
    });
    // info.sender = the CW721 collection contract (MEGA_COLLECTION)
    let info = mock_info(MEGA_COLLECTION, &[]);
    execute(deps.as_mut(), mock_env_at(start), info, receive_msg).unwrap();
}

/// Simulate sending a Standard Mad Scientist NFT to bid on an auction
fn send_bid_nft(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    bidder: &str,
    token_id: &str,
    auction_id: u64,
    time: u64,
) -> Result<cosmwasm_std::Response, ContractError> {
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: bidder.to_string(),
        token_id: token_id.to_string(),
        msg: to_json_binary(&ReceiveNftAction::Bid { auction_id }).unwrap(),
    });
    // info.sender = the CW721 collection contract (MAD_COLLECTION)
    let info = mock_info(MAD_COLLECTION, &[]);
    execute(deps.as_mut(), mock_env_at(time), info, receive_msg)
}

/// Simulate depositing a Standard Mad Scientist NFT for swap staging
fn send_swap_deposit(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    sender: &str,
    token_id: &str,
) {
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: sender.to_string(),
        token_id: token_id.to_string(),
        msg: to_json_binary(&ReceiveNftAction::SwapDeposit).unwrap(),
    });
    let info = mock_info(MAD_COLLECTION, &[]);
    execute(deps.as_mut(), mock_env_at(5000), info, receive_msg).unwrap();
}

fn assert_single_transfer_msg(
    res: &cosmwasm_std::Response,
    expected_contract: &str,
    expected_recipient: &str,
    expected_token_id: &str,
) {
    assert_eq!(res.messages.len(), 1);
    match &res.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            msg,
            funds,
        }) => {
            assert_eq!(contract_addr, expected_contract);
            assert!(funds.is_empty());

            let exec_msg: Cw721ExecuteMsg<Empty, Empty, Empty> = from_json(msg).unwrap();
            match exec_msg {
                Cw721ExecuteMsg::TransferNft {
                    recipient,
                    token_id,
                } => {
                    assert_eq!(recipient, expected_recipient);
                    assert_eq!(token_id, expected_token_id);
                }
                _ => panic!("expected cw721 TransferNft message"),
            }
        }
        _ => panic!("expected wasm execute message"),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// INSTANTIATION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_instantiate_with_defaults() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: None,
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: None,
        anti_snipe_window: None,
        anti_snipe_extension: None,
        max_extension: None,
        max_bidders_per_auction: None,
        max_staging_size: None,
        max_nfts_per_bid: None,
    };
    let info = mock_info(CREATOR, &[]);
    let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
    assert_eq!(res.attributes.len(), 4);

    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.admin, Addr::unchecked(CREATOR));
    assert_eq!(config.default_min_bid, 1);
    assert_eq!(config.anti_snipe_window, 300);
    assert_eq!(config.anti_snipe_extension, 300);
    assert_eq!(config.max_extension, 86400);
    assert_eq!(config.max_bidders_per_auction, 100);
    assert_eq!(config.max_staging_size, 50);
    assert_eq!(config.max_nfts_per_bid, 50);
    assert!(!config.paused);
    assert!(config.pending_admin.is_none());
}

#[test]
fn test_instantiate_with_custom_values() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(5),
        anti_snipe_window: Some(600),
        anti_snipe_extension: Some(120),
        max_extension: Some(43200),
        max_bidders_per_auction: Some(50),
        max_staging_size: Some(25),
        max_nfts_per_bid: Some(50),
    };
    let info = mock_info(CREATOR, &[]);
    instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.admin, Addr::unchecked(ADMIN));
    assert_eq!(config.default_min_bid, 5);
    assert_eq!(config.anti_snipe_window, 600);
    assert_eq!(config.anti_snipe_extension, 120);
    assert_eq!(config.max_extension, 43200);
    assert_eq!(config.max_bidders_per_auction, 50);
    assert_eq!(config.max_staging_size, 25);
}

#[test]
fn test_instantiate_rejects_zero_default_min_bid() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(0),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };

    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_instantiate_rejects_limits_above_cap() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(1_001),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(1_001),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(1_001),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_instantiate_rejects_invalid_anti_snipe_config() {
    let mut deps = mock_dependencies();

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(0),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(0),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(600),
        max_extension: Some(500),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_migrate_entrypoint_works() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let res = migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "action" && a.value == "migrate"));
}

#[test]
fn test_migrate_rejects_wrong_contract_name() {
    let mut deps = mock_dependencies();
    set_contract_version(deps.as_mut().storage, "other-contract", "0.0.1").unwrap();

    let err = migrate(deps.as_mut(), mock_env(), MigrateMsg {}).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// AUCTION CREATION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_create_auction_via_receive_nft() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 2000, 5000, Some(2));

    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(2000),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auction_resp.auction.depositor, Addr::unchecked(ADMIN));
    assert_eq!(auction_resp.auction.mega_token_id, "mega_1");
    assert_eq!(auction_resp.auction.status, AuctionStatus::Active);
    assert_eq!(auction_resp.auction.min_bid, 2);
}

#[test]
fn test_create_auction_rejects_zero_min_bid() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: "mega_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 2000,
            end_time: 5000,
            min_bid: Some(0),
        })
        .unwrap(),
    });
    let info = mock_info(MEGA_COLLECTION, &[]);
    let err = execute(deps.as_mut(), mock_env_at(1000), info, receive_msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_create_auction_unauthorized() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Non-admin tries to deposit a Cosmic NFT
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: BIDDER1.to_string(), // not admin
        token_id: "mega_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 2000,
            end_time: 5000,
            min_bid: None,
        })
        .unwrap(),
    });
    let info = mock_info(MEGA_COLLECTION, &[]);
    let err = execute(deps.as_mut(), mock_env_at(1000), info, receive_msg).unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn test_create_auction_invalid_times() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: "mega_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 5000,
            end_time: 2000, // end before start
            min_bid: None,
        })
        .unwrap(),
    });
    let info = mock_info(MEGA_COLLECTION, &[]);
    let err = execute(deps.as_mut(), mock_env_at(1000), info, receive_msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidAuctionTimes { .. }));
}

#[test]
fn test_create_auction_wrong_collection() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Send from MAD_COLLECTION instead of MEGA_COLLECTION
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: "mega_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 2000,
            end_time: 5000,
            min_bid: None,
        })
        .unwrap(),
    });
    let info = mock_info(MAD_COLLECTION, &[]); // wrong collection!
    let err = execute(deps.as_mut(), mock_env_at(1000), info, receive_msg).unwrap_err();
    assert!(matches!(err, ContractError::UnexpectedCollection { .. }));
}

// FIX #6 TEST: Duplicate Cosmic auction guard
#[test]
fn test_duplicate_mega_auction_blocked() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 2000, 5000, Some(1));

    // Try to create another auction for the same Cosmic token
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: "mega_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 2000,
            end_time: 6000,
            min_bid: None,
        })
        .unwrap(),
    });
    let info = mock_info(MEGA_COLLECTION, &[]);
    let err = execute(deps.as_mut(), mock_env_at(1000), info, receive_msg).unwrap_err();
    assert!(matches!(err, ContractError::DuplicateMegaAuction { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// BIDDING TESTS (all via ReceiveNft — FIX #1/#2)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_bid_via_receive_nft() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Send two NFTs as bids
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();

    // Query the bid
    let bid_resp: BidResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(2000),
            QueryMsg::GetUserBid {
                auction_id: 1,
                bidder: BIDDER1.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(bid_resp.bid.is_some());
    assert_eq!(bid_resp.bid.unwrap().token_ids.len(), 2);
}

#[test]
fn test_bid_wrong_collection() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Send from MEGA_COLLECTION instead of MAD_COLLECTION for a bid
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: BIDDER1.to_string(),
        token_id: "mad_1".to_string(),
        msg: to_json_binary(&ReceiveNftAction::Bid { auction_id: 1 }).unwrap(),
    });
    let info = mock_info(MEGA_COLLECTION, &[]); // wrong!
    let err = execute(deps.as_mut(), mock_env_at(2000), info, receive_msg).unwrap_err();
    assert!(matches!(err, ContractError::UnexpectedCollection { .. }));
}

#[test]
fn test_bid_must_exceed_highest() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Bidder1 sends 3 NFTs
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_3", 1, 2000).unwrap();

    // Bidder2 sends 3 NFTs (equal — accepted into escrow but not highest)
    let res = send_bid_nft(&mut deps, BIDDER2, "mad_4", 1, 2500).unwrap();
    // First NFT: count 1 < 3, not highest
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "false"));

    // Bidder2 sends a 4th NFT to overtake
    send_bid_nft(&mut deps, BIDDER2, "mad_5", 1, 2500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_6", 1, 2500).unwrap();
    let res = send_bid_nft(&mut deps, BIDDER2, "mad_7", 1, 2500).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "true"));
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "total_bid_count" && a.value == "4"));
}

#[test]
fn test_bid_before_start() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 2000, 5000, Some(1));

    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap_err();
    assert!(matches!(err, ContractError::AuctionNotStarted { .. }));
}

#[test]
fn test_bid_exactly_at_start_time_allowed() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 2000, 5000, Some(1));

    let res = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "true"));
}

#[test]
fn test_bid_after_end() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 4000).unwrap_err();
    assert!(matches!(err, ContractError::AuctionAlreadyEnded { .. }));
}

#[test]
fn test_bid_exactly_at_end_time_rejected() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 3000).unwrap_err();
    assert!(matches!(err, ContractError::AuctionAlreadyEnded { .. }));
}

#[test]
fn test_duplicate_token_in_bid() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap_err(); // duplicate
    assert!(matches!(err, ContractError::DuplicateTokenId { .. }));
}

#[test]
fn test_same_token_cannot_be_reused_across_simultaneous_mega_auctions() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));
    send_mega_for_auction(&mut deps, "mega_2", 1000, 5000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 2, 2000).unwrap_err();
    assert_eq!(
        err,
        ContractError::TokenAlreadyEscrowedInAuction {
            token_id: "mad_1".to_string(),
            auction_id: 1,
        }
    );
}

#[test]
fn test_token_lock_applies_across_wallets_and_clears_after_withdrawal() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Auction 1 requires at least 2 NFTs, so a single bid remains non-qualifying.
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(2));
    send_mega_for_auction(&mut deps, "mega_2", 1000, 7000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_lock", 1, 2000).unwrap();

    // Different wallet still cannot reuse the same token while it's escrowed.
    let err = send_bid_nft(&mut deps, BIDDER2, "mad_lock", 2, 2000).unwrap_err();
    assert_eq!(
        err,
        ContractError::TokenAlreadyEscrowedInAuction {
            token_id: "mad_lock".to_string(),
            auction_id: 1,
        }
    );

    // Finalize auction 1 with no qualifying winner, then withdraw to release lock.
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(4100),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();

    // After withdrawal, token can be used again in another auction.
    send_bid_nft(&mut deps, BIDDER1, "mad_lock", 2, 4200).unwrap();
}

#[test]
fn test_same_wallet_can_win_multiple_mega_auctions_with_distinct_bid_tokens() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));
    send_mega_for_auction(&mut deps, "mega_2", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_3", 2, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_4", 2, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_5", 2, 2000).unwrap();

    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 2 },
    )
    .unwrap();

    let auction1: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(4000),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    let auction2: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(4000),
            QueryMsg::GetAuction { auction_id: 2 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        auction1.auction.highest_bidder,
        Some(Addr::unchecked(BIDDER1))
    );
    assert_eq!(
        auction2.auction.highest_bidder,
        Some(Addr::unchecked(BIDDER1))
    );

    let pool: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(4000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool.size, 5);
}

#[test]
fn test_bid_below_minimum_accepted_but_not_highest() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(2));

    // First NFT accepted into escrow but not marked as highest (below min_bid of 2)
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "false"));

    // Second NFT meets min_bid — now becomes highest
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "true"));
}

// ═══════════════════════════════════════════════════════════════════════
// ANTI-SNIPING TESTS (FIX #5: with max extension cap)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_anti_sniping_extends_auction() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Bid at 4800 (within 300s window of 5000 end)
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 4800).unwrap();

    let new_end = res
        .attributes
        .iter()
        .find(|a| a.key == "new_end_time")
        .unwrap()
        .value
        .parse::<u64>()
        .unwrap();
    assert_eq!(new_end, 5100); // 4800 + 300

    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(4800),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auction_resp.auction.end_time, 5100);
    assert_eq!(auction_resp.auction.original_end_time, 5000);
}

#[test]
fn test_anti_snipe_capped_by_max_extension() {
    let mut deps = mock_deps_with_nft_querier();

    // Set up with very short max_extension (600 seconds = 10 min)
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(600), // cap at 10 minutes past original end
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // First snipe bid at 4800: extends to min(4800+300, 5600) = 5100
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 4800).unwrap();

    // Second snipe bid at 5050 (within new end of 5100):
    // extends to min(5050+300, 5600) = 5350
    send_bid_nft(&mut deps, BIDDER2, "mad_2", 1, 5050).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_3", 1, 5050).unwrap(); // overtake

    // Third snipe bid at 5300 (within new end of 5350):
    // would want 5300+300=5600, cap is 5600, so end = 5600
    send_bid_nft(&mut deps, BIDDER1, "mad_4", 1, 5300).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_5", 1, 5300).unwrap();
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_6", 1, 5300).unwrap(); // overtake at 4

    let new_end = res
        .attributes
        .iter()
        .find(|a| a.key == "new_end_time")
        .unwrap()
        .value
        .parse::<u64>()
        .unwrap();
    assert_eq!(new_end, 5600); // Capped at original_end_time + max_extension
}

#[test]
fn test_no_extension_outside_window() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Bid well before snipe window
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let new_end = res
        .attributes
        .iter()
        .find(|a| a.key == "new_end_time")
        .unwrap()
        .value
        .parse::<u64>()
        .unwrap();
    assert_eq!(new_end, 5000); // Unchanged
}

#[test]
fn test_anti_snipe_triggers_at_exact_window_boundary() {
    let mut deps = mock_deps_with_nft_querier();

    // Use extension > window so boundary-triggered extension is observable.
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(600),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // now + window == end_time, should still trigger anti-snipe extension
    let res = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 4700).unwrap();

    let new_end = res
        .attributes
        .iter()
        .find(|a| a.key == "new_end_time")
        .unwrap()
        .value
        .parse::<u64>()
        .unwrap();
    assert_eq!(new_end, 5300);
}

// ═══════════════════════════════════════════════════════════════════════
// FINALIZE AUCTION TESTS (FIX #4: self-claim pattern)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_finalize_with_winner_goes_to_finalizing() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    // Bidder1 and Bidder2 bid
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_3", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_4", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_5", 1, 2000).unwrap(); // Bidder2 leads with 3

    // Finalize
    let env = mock_env_at(4000);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "winner" && a.value == BIDDER2));

    // Status should be Finalizing (losers haven't withdrawn yet)
    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            env.clone(),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auction_resp.auction.status, AuctionStatus::Finalizing);

    // Winner's NFTs should be in pool (FIX #8: check via counter)
    let pool: PoolSizeResponse =
        from_json(query(deps.as_ref(), env, QueryMsg::GetPoolSize {}).unwrap()).unwrap();
    assert_eq!(pool.size, 3);
}

#[test]
fn test_finalize_no_bids_returns_mega() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    let env = mock_env_at(4000);
    let res = execute(
        deps.as_mut(),
        env.clone(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "winner" && a.value == "none"));
    assert_single_transfer_msg(&res, MEGA_COLLECTION, ADMIN, "mega_1");

    let auction_resp: AuctionResponse =
        from_json(query(deps.as_ref(), env, QueryMsg::GetAuction { auction_id: 1 }).unwrap())
            .unwrap();
    assert_eq!(auction_resp.auction.status, AuctionStatus::Completed);
}

#[test]
fn test_finalize_sub_minimum_bid_allows_withdraw_after_completed() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(2));

    // Escrow one NFT below min bid (accepted but not highest)
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();

    // Finalize with no qualifying winner.
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Bidder can still recover escrow even though auction status is Completed.
    let withdraw = execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();
    assert_eq!(withdraw.messages.len(), 1);
}

#[test]
fn test_finalize_no_bids_returns_mega_to_original_depositor_after_admin_transfer() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    // Transfer admin from ADMIN to BIDDER1 before finalization.
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ProposeAdmin {
            new_admin: BIDDER1.to_string(),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(1600),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::AcceptAdmin {},
    )
    .unwrap();

    let res = execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(BIDDER1, &[]), // new admin finalizes
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Cosmic must return to original depositor (ADMIN), not current admin.
    assert_single_transfer_msg(&res, MEGA_COLLECTION, ADMIN, "mega_1");
}

#[test]
fn test_finalize_before_end_fails() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    let err = execute(
        deps.as_mut(),
        mock_env_at(2000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::AuctionNotEnded { .. }));
}

#[test]
fn test_finalize_exactly_at_end_time_allowed() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let res = execute(
        deps.as_mut(),
        mock_env_at(3000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "action" && a.value == "finalize_auction"));
}

// ═══════════════════════════════════════════════════════════════════════
// WITHDRAW BID TESTS (FIX #4)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_loser_withdraws_after_finalize() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_3", 1, 2000).unwrap(); // Bidder2 wins

    // Finalize
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Bidder1 withdraws their escrowed NFT
    let res = execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "nfts_returned" && a.value == "1"));
    assert_eq!(res.messages.len(), 1); // One TransferNft back to Bidder1
}

#[test]
fn test_withdraw_not_allowed_during_active_auction() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env_at(2500),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::WithdrawNotAllowed { .. }));
}

#[test]
fn test_winner_cannot_withdraw() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap(); // only bidder = winner

    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Winner's escrow was already cleared during finalize
    let err = execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NothingToWithdraw { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// CANCEL AUCTION TESTS (FIX #7: only before bids)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_cancel_before_bids() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    let res = execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap();

    assert_single_transfer_msg(&res, MEGA_COLLECTION, ADMIN, "mega_1");

    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(1500),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auction_resp.auction.status, AuctionStatus::Cancelled);
}

#[test]
fn test_cancel_returns_mega_to_original_depositor_after_admin_transfer() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Transfer admin from ADMIN to BIDDER1.
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ProposeAdmin {
            new_admin: BIDDER1.to_string(),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(1600),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::AcceptAdmin {},
    )
    .unwrap();

    // New admin can cancel, but NFT must go back to original depositor.
    let res = execute(
        deps.as_mut(),
        mock_env_at(1700),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap();

    assert_single_transfer_msg(&res, MEGA_COLLECTION, ADMIN, "mega_1");
}

#[test]
fn test_cancel_with_bids_fails() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env_at(2500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::CannotCancelWithBids { .. }));
}

#[test]
fn test_cancel_unauthorized() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    let err = execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

// ═══════════════════════════════════════════════════════════════════════
// SWAP POOL TESTS (FIX #3: two-step deposit + claim)
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_swap_deposit_and_claim() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Populate pool via auction
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_3", 1, 1500).unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Pool should have 3 NFTs
    let pool: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(4000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool.size, 3);

    // Step 1: Bidder2 deposits 2 NFTs for swapping
    send_swap_deposit(&mut deps, BIDDER2, "mad_10");
    send_swap_deposit(&mut deps, BIDDER2, "mad_11");

    // Check staging
    let staging: SwapStagingResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(5000),
            QueryMsg::GetSwapStaging {
                user: BIDDER2.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(staging.token_ids.len(), 2);

    // Step 2: Claim swap — pick 2 from pool
    let res = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER2, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_1".to_string(), "mad_2".to_string()],
        },
    )
    .unwrap();

    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "swap_count" && a.value == "2"));

    // Pool size unchanged (2 in, 2 out)
    let pool: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(5000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool.size, 3);
}

#[test]
fn test_swap_count_mismatch() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Populate pool
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 1500).unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Deposit 1 but try to claim 2
    send_swap_deposit(&mut deps, BIDDER2, "mad_10");

    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER2, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_1".to_string(), "mad_2".to_string()],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::SwapCountMismatch { .. }));
}

#[test]
fn test_swap_no_staged_tokens() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_1".to_string()],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NoStagedTokens {}));
}

#[test]
fn test_swap_overlap() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Populate pool with "mad_1"
    crate::state::POOL
        .save(deps.as_mut().storage, "mad_1", &())
        .unwrap();
    crate::state::POOL_SIZE
        .save(deps.as_mut().storage, &1u64)
        .unwrap();

    // Deposit "mad_1" for swap (same as what's in pool)
    send_swap_deposit(&mut deps, BIDDER1, "mad_1");

    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_1".to_string()],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::SwapOverlap { .. }));
}

#[test]
fn test_swap_token_not_in_pool() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_swap_deposit(&mut deps, BIDDER1, "mad_10");

    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_99".to_string()],
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::TokenNotInPool { .. }));
}

#[test]
fn test_withdraw_staged_returns_tokens_and_clears_staging() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_swap_deposit(&mut deps, BIDDER1, "mad_10");
    send_swap_deposit(&mut deps, BIDDER1, "mad_11");

    let res = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawStaged {},
    )
    .unwrap();

    assert_eq!(res.messages.len(), 2);
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "action" && a.value == "withdraw_staged"));

    let staging: SwapStagingResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(5000),
            QueryMsg::GetSwapStaging {
                user: BIDDER1.to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(staging.token_ids.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// UPDATE CONFIG TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_update_config() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    let update_msg = ExecuteMsg::UpdateConfig {
        default_min_bid: Some(5),
        anti_snipe_window: Some(600),
        anti_snipe_extension: None,
        max_extension: Some(43200),
        max_bidders_per_auction: Some(200),
        max_staging_size: Some(100),
        max_nfts_per_bid: Some(50),
    };
    execute(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), update_msg).unwrap();

    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.admin, Addr::unchecked(ADMIN)); // unchanged (no admin field in UpdateConfig)
    assert_eq!(config.default_min_bid, 5);
    assert_eq!(config.anti_snipe_window, 600);
    assert_eq!(config.anti_snipe_extension, 300); // unchanged
    assert_eq!(config.max_extension, 43200);
    assert_eq!(config.max_bidders_per_auction, 200);
    assert_eq!(config.max_staging_size, 100);
}

#[test]
fn test_update_config_unauthorized() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: Some(99),
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn test_update_config_rejects_zero_default_min_bid() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: Some(0),
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_update_config_rejects_zero_limits() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: Some(0),
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: Some(0),
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: Some(0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_update_config_rejects_limits_above_cap() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: Some(1_001),
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: Some(1_001),
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: Some(1_001),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

#[test]
fn test_update_config_rejects_invalid_anti_snipe_config() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: Some(0),
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: Some(0),
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: Some(300),
            anti_snipe_extension: Some(600),
            max_extension: Some(500),
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// QUERY TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_query_all_auctions_with_filter() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));
    send_mega_for_auction(&mut deps, "mega_2", 1000, 3000, Some(1));

    // Cancel first (no bids so allowed)
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap();

    let active: AuctionsResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(2000),
            QueryMsg::GetAllAuctions {
                status: Some(AuctionStatus::Active),
                start_after: None,
                limit: None,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(active.auctions.len(), 1);
    assert_eq!(active.auctions[0].auction_id, 2);

    let all: AuctionsResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(2000),
            QueryMsg::GetAllAuctions {
                status: None,
                start_after: None,
                limit: None,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(all.auctions.len(), 2);
}

#[test]
fn test_query_all_auctions_propagates_deserialization_errors() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));
    send_mega_for_auction(&mut deps, "mega_2", 1000, 3000, Some(1));

    let bad_key = AUCTIONS.key(2u64);
    deps.storage.set(&bad_key, b"not-json");

    let err = query(
        deps.as_ref(),
        mock_env_at(2000),
        QueryMsg::GetAllAuctions {
            status: None,
            start_after: None,
            limit: None,
        },
    )
    .unwrap_err();
    assert!(!err.to_string().is_empty());
}

#[test]
fn test_query_auction_bid_list_is_bounded() {
    let mut deps = mock_deps_with_nft_querier();

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(200),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    for i in 0..120 {
        let bidder = MockApi::default().addr_make(format!("bounded-bidder-{i}").as_str());
        let token_id = format!("mad-{i}");
        send_bid_nft(&mut deps, bidder.as_str(), token_id.as_str(), 1, 2000).unwrap();
    }

    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(2500),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(auction_resp.bids.len(), 100);
}

#[test]
fn test_query_pool_contents() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // Manually add tokens to pool
    crate::state::POOL
        .save(deps.as_mut().storage, "mad_1", &())
        .unwrap();
    crate::state::POOL
        .save(deps.as_mut().storage, "mad_2", &())
        .unwrap();
    crate::state::POOL
        .save(deps.as_mut().storage, "mad_3", &())
        .unwrap();
    crate::state::POOL_SIZE
        .save(deps.as_mut().storage, &3u64)
        .unwrap();

    let pool: PoolContentsResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(1000),
            QueryMsg::GetPoolContents {
                start_after: None,
                limit: Some(2),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(pool.token_ids.len(), 2);

    let pool_size: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(1000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool_size.size, 3);
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: PAUSE MECHANISM TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_pause_blocks_receive_nft() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Admin pauses
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::SetPaused { paused: true },
    )
    .unwrap();

    // Verify paused in config
    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env_at(1500), QueryMsg::GetConfig {}).unwrap())
            .unwrap();
    assert!(config.paused);

    // Bid should be rejected
    let err = send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap_err();
    assert_eq!(err, ContractError::ContractPaused {});

    // Swap deposit should be rejected
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: BIDDER1.to_string(),
        token_id: "mad_2".to_string(),
        msg: to_json_binary(&ReceiveNftAction::SwapDeposit).unwrap(),
    });
    let err = execute(
        deps.as_mut(),
        mock_env_at(2000),
        mock_info(MAD_COLLECTION, &[]),
        receive_msg,
    )
    .unwrap_err();
    assert_eq!(err, ContractError::ContractPaused {});
}

#[test]
fn test_unpause_restores_operations() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Pause then unpause
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::SetPaused { paused: true },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env_at(1600),
        mock_info(ADMIN, &[]),
        ExecuteMsg::SetPaused { paused: false },
    )
    .unwrap();

    // Bidding should work again
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
}

#[test]
fn test_pause_unauthorized() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    let err = execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::SetPaused { paused: true },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn test_pause_does_not_block_withdrawals() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_3", 1, 2000).unwrap();

    // Finalize
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Pause the contract
    execute(
        deps.as_mut(),
        mock_env_at(4100),
        mock_info(ADMIN, &[]),
        ExecuteMsg::SetPaused { paused: true },
    )
    .unwrap();

    // Withdrawal should still work (not gated by pause)
    let res = execute(
        deps.as_mut(),
        mock_env_at(4200),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();
    assert_eq!(res.messages.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: TWO-STEP ADMIN TRANSFER TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_two_step_admin_transfer() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    // Step 1: Admin proposes new admin
    execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ProposeAdmin {
            new_admin: BIDDER1.to_string(),
        },
    )
    .unwrap();

    // Verify pending_admin set
    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.admin, Addr::unchecked(ADMIN)); // Still old admin
    assert_eq!(config.pending_admin, Some(Addr::unchecked(BIDDER1)));

    // Step 2: New admin accepts
    execute(
        deps.as_mut(),
        mock_env(),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::AcceptAdmin {},
    )
    .unwrap();

    // Verify transfer complete
    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.admin, Addr::unchecked(BIDDER1));
    assert!(config.pending_admin.is_none());
}

#[test]
fn test_propose_admin_unauthorized() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::ProposeAdmin {
            new_admin: BIDDER2.to_string(),
        },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

#[test]
fn test_accept_admin_wrong_sender() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    // Propose BIDDER1
    execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ProposeAdmin {
            new_admin: BIDDER1.to_string(),
        },
    )
    .unwrap();

    // BIDDER2 tries to accept (wrong person)
    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(BIDDER2, &[]),
        ExecuteMsg::AcceptAdmin {},
    )
    .unwrap_err();
    assert_eq!(err, ContractError::NotPendingAdmin {});
}

#[test]
fn test_accept_admin_no_pending() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env(),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::AcceptAdmin {},
    )
    .unwrap_err();
    assert_eq!(err, ContractError::NoPendingAdmin {});
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: MAX BIDDERS PER AUCTION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_max_bidders_cap() {
    let mut deps = mock_deps_with_nft_querier();

    // Set max_bidders_per_auction to 2
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(2),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // First two bidders succeed
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_2", 1, 2000).unwrap();

    // Third bidder rejected
    let err = send_bid_nft(&mut deps, BIDDER3, "mad_3", 1, 2000).unwrap_err();
    assert!(matches!(err, ContractError::MaxBiddersReached { .. }));

    // Existing bidders can still add more NFTs
    send_bid_nft(&mut deps, BIDDER1, "mad_4", 1, 2000).unwrap();
}

#[test]
fn test_max_bidders_zero_rejected() {
    let mut deps = mock_deps_with_nft_querier();

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(0),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err =
        instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: FORCE COMPLETE AUCTION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_force_complete_finalizing_auction() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_3", 1, 2000).unwrap();

    // Finalize → Finalizing
    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    // Force complete
    execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ForceCompleteAuction { auction_id: 1 },
    )
    .unwrap();

    let auction_resp: AuctionResponse = from_json(
        query(
            deps.as_ref(),
            mock_env_at(5000),
            QueryMsg::GetAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auction_resp.auction.status, AuctionStatus::Completed);

    // Loser can still withdraw after force-complete (Completed is allowed).
    let withdraw = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();
    assert_eq!(withdraw.messages.len(), 1);
}

#[test]
fn test_force_complete_not_finalizing_fails() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // Active auction — can't force complete
    let err = execute(
        deps.as_mut(),
        mock_env_at(2000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::ForceCompleteAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::NotFinalizing { .. }));
}

#[test]
fn test_force_complete_unauthorized() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 3000, Some(1));

    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 1500).unwrap();

    execute(
        deps.as_mut(),
        mock_env_at(4000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();

    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::ForceCompleteAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::Unauthorized {});
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: SWAP STAGING SIZE CAP TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_staging_size_cap() {
    let mut deps = mock_deps_with_nft_querier();

    // Set max_staging_size to 2
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(2),
        max_nfts_per_bid: Some(50),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_swap_deposit(&mut deps, BIDDER1, "mad_1");
    send_swap_deposit(&mut deps, BIDDER1, "mad_2");

    // Third deposit rejected
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: BIDDER1.to_string(),
        token_id: "mad_3".to_string(),
        msg: to_json_binary(&ReceiveNftAction::SwapDeposit).unwrap(),
    });
    let err = execute(
        deps.as_mut(),
        mock_env_at(5000),
        mock_info(MAD_COLLECTION, &[]),
        receive_msg,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::StagingLimitReached { .. }));
}

#[test]
fn test_staging_size_zero_rejected() {
    let mut deps = mock_deps_with_nft_querier();

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(0),
        max_nfts_per_bid: Some(50),
    };
    let err =
        instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: TOTAL_BIDDERS COUNTER FIX — CANCEL GUARD
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_cancel_blocked_even_with_sub_minimum_bids() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(3));

    // Bid below minimum (only 1 NFT, min is 3) — accepted into escrow
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    // Cancel should fail because total_bidders > 0
    let err = execute(
        deps.as_mut(),
        mock_env_at(2500),
        mock_info(ADMIN, &[]),
        ExecuteMsg::CancelAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::CannotCancelWithBids { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: FUNDS REJECTION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_funds_rejected_on_execute() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    // Attaching native tokens to any execute message should fail
    let info = mock_info(ADMIN, &[cosmwasm_std::coin(1000, "uatom")]);
    let err = execute(
        deps.as_mut(),
        mock_env_at(2000),
        info,
        ExecuteMsg::SetPaused { paused: true },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::FundsNotAllowed {});
}

#[test]
fn test_funds_rejected_on_finalize() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();

    let info = mock_info(BIDDER1, &[cosmwasm_std::coin(500, "uatom")]);
    let err = execute(
        deps.as_mut(),
        mock_env_at(6000),
        info,
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap_err();
    assert_eq!(err, ContractError::FundsNotAllowed {});
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: COLLECTION ADDRESS COLLISION TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_collection_address_collision_rejected() {
    let mut deps = mock_dependencies();
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MAD_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(50),
    };
    let err = instantiate(deps.as_mut(), mock_env(), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert_eq!(err, ContractError::CollectionAddressCollision {});
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: PER-BIDDER ESCROW CAP TESTS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_per_bidder_escrow_cap() {
    let mut deps = mock_deps_with_nft_querier();

    // Set max_nfts_per_bid to 2
    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(2),
    };
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(1));

    // First two NFTs succeed
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();

    // Third NFT from same bidder rejected
    let err = send_bid_nft(&mut deps, BIDDER1, "mad_3", 1, 2000).unwrap_err();
    assert!(matches!(err, ContractError::MaxEscrowPerBidder { .. }));

    // Different bidder can still bid
    send_bid_nft(&mut deps, BIDDER2, "mad_4", 1, 2000).unwrap();
}

#[test]
fn test_per_bidder_escrow_zero_rejected() {
    let mut deps = mock_deps_with_nft_querier();

    let msg = InstantiateMsg {
        admin: Some(ADMIN.to_string()),
        mad_scientist_collection: MAD_COLLECTION.to_string(),
        mega_mad_scientist_collection: MEGA_COLLECTION.to_string(),
        default_min_bid: Some(1),
        anti_snipe_window: Some(300),
        anti_snipe_extension: Some(300),
        max_extension: Some(86400),
        max_bidders_per_auction: Some(100),
        max_staging_size: Some(50),
        max_nfts_per_bid: Some(0),
    };
    let err =
        instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap_err();
    assert!(matches!(err, ContractError::InvalidConfig { .. }));
}

// ═══════════════════════════════════════════════════════════════════════
// HARDENING: UPDATE CONFIG WITH NEW FIELDS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_update_config_max_nfts_per_bid() {
    let mut deps = mock_dependencies();
    setup_contract(&mut deps);

    // Update max_nfts_per_bid
    execute(
        deps.as_mut(),
        mock_env(),
        mock_info(ADMIN, &[]),
        ExecuteMsg::UpdateConfig {
            default_min_bid: None,
            anti_snipe_window: None,
            anti_snipe_extension: None,
            max_extension: None,
            max_bidders_per_auction: None,
            max_staging_size: None,
            max_nfts_per_bid: Some(10),
        },
    )
    .unwrap();

    let config: ConfigResponse =
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap()).unwrap();
    assert_eq!(config.max_nfts_per_bid, 10);
}

// ═══════════════════════════════════════════════════════════════════════
// INTEGRATION TEST: FULL FLOW
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn test_full_auction_to_pool_to_swap_flow() {
    let mut deps = mock_deps_with_nft_querier();
    setup_contract(&mut deps);

    // 1. Admin deposits Cosmic NFT to create auction
    send_mega_for_auction(&mut deps, "mega_1", 1000, 5000, Some(2));

    // 2. Bidder1 bids 3 NFTs (one at a time via CW721 Send)
    send_bid_nft(&mut deps, BIDDER1, "mad_1", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_2", 1, 2000).unwrap();
    send_bid_nft(&mut deps, BIDDER1, "mad_3", 1, 2000).unwrap();

    // 3. Bidder2 bids 5 NFTs
    send_bid_nft(&mut deps, BIDDER2, "mad_4", 1, 2500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_5", 1, 2500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_6", 1, 2500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_7", 1, 2500).unwrap();
    send_bid_nft(&mut deps, BIDDER2, "mad_8", 1, 2500).unwrap();

    // 4. Bidder3 bids 5 NFTs (equal — does NOT overtake Bidder2)
    send_bid_nft(&mut deps, BIDDER3, "mad_9", 1, 3000).unwrap();
    send_bid_nft(&mut deps, BIDDER3, "mad_10", 1, 3000).unwrap();
    send_bid_nft(&mut deps, BIDDER3, "mad_11", 1, 3000).unwrap();
    send_bid_nft(&mut deps, BIDDER3, "mad_12", 1, 3000).unwrap();
    let res = send_bid_nft(&mut deps, BIDDER3, "mad_13", 1, 3000).unwrap();
    assert!(res
        .attributes
        .iter()
        .any(|a| a.key == "is_highest" && a.value == "false"));

    // 5. Finalize auction
    let finalize_res = execute(
        deps.as_mut(),
        mock_env_at(6000),
        mock_info(ADMIN, &[]),
        ExecuteMsg::FinalizeAuction { auction_id: 1 },
    )
    .unwrap();
    assert!(finalize_res
        .attributes
        .iter()
        .any(|a| a.key == "winner" && a.value == BIDDER2));

    // 6. Pool should contain Bidder2's 5 NFTs
    let pool_size: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(6000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool_size.size, 5);

    // 7. Losers withdraw their NFTs
    let w1 = execute(
        deps.as_mut(),
        mock_env_at(6000),
        mock_info(BIDDER1, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();
    assert_eq!(w1.messages.len(), 3); // 3 NFTs returned

    let w3 = execute(
        deps.as_mut(),
        mock_env_at(6000),
        mock_info(BIDDER3, &[]),
        ExecuteMsg::WithdrawBid { auction_id: 1 },
    )
    .unwrap();
    assert_eq!(w3.messages.len(), 5); // 5 NFTs returned

    // 8. Someone swaps 2 NFTs from the pool
    send_swap_deposit(&mut deps, BIDDER3, "mad_20");
    send_swap_deposit(&mut deps, BIDDER3, "mad_21");

    let swap_res = execute(
        deps.as_mut(),
        mock_env_at(7000),
        mock_info(BIDDER3, &[]),
        ExecuteMsg::ClaimSwap {
            requested_ids: vec!["mad_4".to_string(), "mad_5".to_string()],
        },
    )
    .unwrap();
    assert!(swap_res
        .attributes
        .iter()
        .any(|a| a.key == "swap_count" && a.value == "2"));

    // Pool size unchanged
    let pool_size: PoolSizeResponse =
        from_json(query(deps.as_ref(), mock_env_at(7000), QueryMsg::GetPoolSize {}).unwrap())
            .unwrap();
    assert_eq!(pool_size.size, 5);
}
