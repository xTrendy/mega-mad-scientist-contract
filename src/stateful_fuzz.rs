use cosmwasm_std::testing::{mock_env, mock_info, MockApi, MockQuerier, MockStorage};
use cosmwasm_std::{
    from_json, to_json_binary, Addr, ContractResult, Empty, Env, OwnedDeps, QuerierResult,
    SystemError, SystemResult, Timestamp, WasmQuery,
};
use cw721::Cw721ReceiveMsg;
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::contract::{execute, instantiate, query};
use crate::error::ContractError;
use crate::msg::{
    AuctionResponse, BidResponse, ExecuteMsg, InstantiateMsg, PoolContentsResponse,
    PoolSizeResponse, QueryMsg, ReceiveNftAction,
};
use crate::state::AuctionStatus;

const ADMIN: &str = "cosmos1admin";
const BIDDER1: &str = "cosmos1bidder1";
const BIDDER2: &str = "cosmos1bidder2";
const BIDDER3: &str = "cosmos1bidder3";
const MAD_COLLECTION: &str = "cosmos1mad_collection";
const MEGA_COLLECTION: &str = "cosmos1mega_collection";
const CONTRACT: &str = "cosmos1contract";
const AUCTION_ID: u64 = 1;

const BIDDERS: [&str; 3] = [BIDDER1, BIDDER2, BIDDER3];

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
                let query_msg: Result<cw721::Cw721QueryMsg, _> = from_json(msg);
                match query_msg {
                    Ok(cw721::Cw721QueryMsg::OwnerOf { .. }) => {
                        let resp = cw721::OwnerOfResponse {
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

fn instantiate_with_min_bid(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    min_bid: u64,
) {
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
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: ADMIN.to_string(),
        token_id: "mega_fuzz".to_string(),
        msg: to_json_binary(&ReceiveNftAction::DepositMega {
            start_time: 1000,
            end_time: 3000,
            min_bid: Some(min_bid),
        })
        .unwrap(),
    });
    execute(
        deps.as_mut(),
        mock_env_at(1000),
        mock_info(MEGA_COLLECTION, &[]),
        receive_msg,
    )
    .unwrap();
}

fn instantiate_three_auctions(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    min_bid: u64,
) {
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
    instantiate(deps.as_mut(), mock_env_at(1000), mock_info(ADMIN, &[]), msg).unwrap();

    for auction_id in 1_u64..=3_u64 {
        let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
            sender: ADMIN.to_string(),
            token_id: format!("mega_fuzz_{auction_id}"),
            msg: to_json_binary(&ReceiveNftAction::DepositMega {
                start_time: 1000,
                end_time: 3000,
                min_bid: Some(min_bid),
            })
            .unwrap(),
        });
        execute(
            deps.as_mut(),
            mock_env_at(1000),
            mock_info(MEGA_COLLECTION, &[]),
            receive_msg,
        )
        .unwrap();
    }
}

fn send_bid_nft(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    bidder: &str,
    token_id: String,
) -> Result<(), ContractError> {
    send_bid_nft_for_auction(deps, bidder, AUCTION_ID, token_id)
}

fn send_bid_nft_for_auction(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier, Empty>,
    bidder: &str,
    auction_id: u64,
    token_id: String,
) -> Result<(), ContractError> {
    let receive_msg = ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
        sender: bidder.to_string(),
        token_id,
        msg: to_json_binary(&ReceiveNftAction::Bid { auction_id }).unwrap(),
    });
    execute(
        deps.as_mut(),
        mock_env_at(1500),
        mock_info(MAD_COLLECTION, &[]),
        receive_msg,
    )?;
    Ok(())
}

fn expected_highest(step_bidders: &[usize], min_bid: u64) -> (Option<usize>, u64) {
    let mut counts = [0_u64; 3];
    let mut highest_bidder: Option<usize> = None;
    let mut highest_count = 0_u64;

    for bidder_idx in step_bidders {
        counts[*bidder_idx] += 1;
        let total_bid_count = counts[*bidder_idx];
        let meets_minimum = total_bid_count >= min_bid;
        let is_same_bidder = highest_bidder == Some(*bidder_idx);
        let exceeds_highest = total_bid_count > highest_count || highest_bidder.is_none();
        let qualifies_as_highest = meets_minimum && (exceeds_highest || is_same_bidder);

        if qualifies_as_highest {
            highest_count = total_bid_count;
            highest_bidder = Some(*bidder_idx);
        }
    }

    (highest_bidder, highest_count)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        max_local_rejects: 2048,
        .. ProptestConfig::default()
    })]

    #[test]
    fn proptest_stateful_auction_invariants(
        min_bid in 1_u64..=4,
        steps in prop::collection::vec(0_usize..3, 1..40),
        force_complete in any::<bool>(),
    ) {
        let mut deps = mock_deps_with_nft_querier();
        instantiate_with_min_bid(&mut deps, min_bid);

        let mut model_counts = [0_u64; 3];
        let mut executed_step_bidders: Vec<usize> = vec![];

        for (i, bidder_idx) in steps.iter().enumerate() {
            let token_id = format!("fuzz_bid_{i}");
            let bidder = BIDDERS[*bidder_idx];
            let bid_result = send_bid_nft(&mut deps, bidder, token_id);
            prop_assert!(bid_result.is_ok());

            model_counts[*bidder_idx] += 1;
            executed_step_bidders.push(*bidder_idx);

            let (expected_winner_idx, expected_highest_count) =
                expected_highest(&executed_step_bidders, min_bid);

            let auction_resp: AuctionResponse = from_json(
                query(
                    deps.as_ref(),
                    mock_env_at(1500),
                    QueryMsg::GetAuction {
                        auction_id: AUCTION_ID,
                    },
                )
                .unwrap(),
            )
            .unwrap();

            prop_assert_eq!(auction_resp.auction.highest_bid_count, expected_highest_count);
            match expected_winner_idx {
                Some(idx) => {
                    prop_assert_eq!(
                        auction_resp.auction.highest_bidder,
                        Some(Addr::unchecked(BIDDERS[idx])),
                    );
                }
                None => {
                    prop_assert!(auction_resp.auction.highest_bidder.is_none());
                }
            }

            let mut seen = HashSet::new();
            for bid in &auction_resp.bids {
                for tid in &bid.token_ids {
                    prop_assert!(seen.insert(tid.clone()));
                }
            }

            for idx in 0..3 {
                let bid_resp: BidResponse = from_json(
                    query(
                        deps.as_ref(),
                        mock_env_at(1500),
                        QueryMsg::GetUserBid {
                            auction_id: AUCTION_ID,
                            bidder: BIDDERS[idx].to_string(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();

                if model_counts[idx] == 0 {
                    prop_assert!(bid_resp.bid.is_none());
                } else {
                    let bid = bid_resp.bid.unwrap();
                    prop_assert_eq!(bid.bidder, Addr::unchecked(BIDDERS[idx]));
                    prop_assert_eq!(bid.token_ids.len() as u64, model_counts[idx]);
                }
            }
        }

        let (expected_winner_idx, expected_highest_count) =
            expected_highest(&executed_step_bidders, min_bid);

        let finalize = execute(
            deps.as_mut(),
            mock_env_at(4000),
            mock_info(ADMIN, &[]),
            ExecuteMsg::FinalizeAuction {
                auction_id: AUCTION_ID,
            },
        );
        prop_assert!(finalize.is_ok());

        let mut auction_resp: AuctionResponse = from_json(
            query(
                deps.as_ref(),
                mock_env_at(4000),
                QueryMsg::GetAuction {
                    auction_id: AUCTION_ID,
                },
            )
            .unwrap(),
        )
        .unwrap();

        if expected_winner_idx.is_some() {
            prop_assert_eq!(auction_resp.auction.status, AuctionStatus::Finalizing);
        } else {
            prop_assert_eq!(auction_resp.auction.status, AuctionStatus::Completed);
        }

        prop_assert_eq!(auction_resp.auction.highest_bid_count, expected_highest_count);

        if force_complete && expected_winner_idx.is_some() {
            let force_res = execute(
                deps.as_mut(),
                mock_env_at(4100),
                mock_info(ADMIN, &[]),
                ExecuteMsg::ForceCompleteAuction {
                    auction_id: AUCTION_ID,
                },
            );
            prop_assert!(force_res.is_ok());

            auction_resp = from_json(
                query(
                    deps.as_ref(),
                    mock_env_at(4100),
                    QueryMsg::GetAuction {
                        auction_id: AUCTION_ID,
                    },
                )
                .unwrap(),
            )
            .unwrap();
            prop_assert_eq!(auction_resp.auction.status, AuctionStatus::Completed);
        }

        for idx in 0..3 {
            let bidder = BIDDERS[idx];
            let withdraw = execute(
                deps.as_mut(),
                mock_env_at(4200),
                mock_info(bidder, &[]),
                ExecuteMsg::WithdrawBid {
                    auction_id: AUCTION_ID,
                },
            );

            let is_winner = expected_winner_idx == Some(idx);
            let count = model_counts[idx];

            if is_winner || count == 0 {
                match withdraw {
                    Err(ContractError::NothingToWithdraw { .. }) => {}
                    other => {
                        prop_assert!(false, "expected NothingToWithdraw, got: {:?}", other);
                    }
                }
            } else {
                let res = withdraw.unwrap();
                prop_assert_eq!(res.messages.len() as u64, count);
            }
        }

        for bidder in &BIDDERS {
            let second_withdraw = execute(
                deps.as_mut(),
                mock_env_at(4300),
                mock_info(bidder, &[]),
                ExecuteMsg::WithdrawBid {
                    auction_id: AUCTION_ID,
                },
            );
            match second_withdraw {
                Err(ContractError::NothingToWithdraw { .. }) => {}
                other => {
                    prop_assert!(
                        false,
                        "expected second withdraw to fail with NothingToWithdraw, got: {:?}",
                        other
                    );
                }
            }
        }

        let pool_size: PoolSizeResponse = from_json(
            query(deps.as_ref(), mock_env_at(4300), QueryMsg::GetPoolSize {}).unwrap(),
        )
        .unwrap();

        let pool_contents: PoolContentsResponse = from_json(
            query(
                deps.as_ref(),
                mock_env_at(4300),
                QueryMsg::GetPoolContents {
                    start_after: None,
                    limit: Some(100),
                },
            )
            .unwrap(),
        )
        .unwrap();
        let unique_pool_tokens: HashSet<String> = pool_contents.token_ids.iter().cloned().collect();
        prop_assert_eq!(unique_pool_tokens.len(), pool_contents.token_ids.len());
        prop_assert_eq!(pool_contents.token_ids.len() as u64, pool_size.size);

        let expected_pool_size = expected_winner_idx.map(|idx| model_counts[idx]).unwrap_or(0);
        prop_assert_eq!(pool_size.size, expected_pool_size);

        let final_auction_resp: AuctionResponse = from_json(
            query(
                deps.as_ref(),
                mock_env_at(4300),
                QueryMsg::GetAuction {
                    auction_id: AUCTION_ID,
                },
            )
            .unwrap(),
        )
        .unwrap();
        prop_assert!(final_auction_resp.bids.is_empty());
    }

    #[test]
    fn proptest_multi_auction_token_lock_invariants(
        steps in prop::collection::vec((0_usize..3, 1_u64..=3, 0_u16..40), 1..80),
    ) {
        let mut deps = mock_deps_with_nft_querier();
        instantiate_three_auctions(&mut deps, 1);

        let mut model_locks: HashMap<String, u64> = HashMap::new();

        for (bidder_idx, auction_id, token_idx) in &steps {
            let bidder = BIDDERS[*bidder_idx];
            let token_id = format!("shared_{token_idx}");
            let bid_result = send_bid_nft_for_auction(
                &mut deps,
                bidder,
                *auction_id,
                token_id.clone(),
            );

            match model_locks.get(&token_id) {
                Some(existing_auction_id) if *existing_auction_id != *auction_id => {
                    match bid_result {
                        Err(ContractError::TokenAlreadyEscrowedInAuction { token_id: err_tid, auction_id: err_aid }) => {
                            prop_assert_eq!(err_tid, token_id);
                            prop_assert_eq!(err_aid, *existing_auction_id);
                        }
                        other => {
                            prop_assert!(false, "expected TokenAlreadyEscrowedInAuction, got: {:?}", other);
                        }
                    }
                }
                Some(_) => {
                    match bid_result {
                        Err(ContractError::DuplicateTokenId { token_id: err_tid }) => {
                            prop_assert_eq!(err_tid, token_id);
                        }
                        other => {
                            prop_assert!(false, "expected DuplicateTokenId, got: {:?}", other);
                        }
                    }
                }
                None => {
                    prop_assert!(bid_result.is_ok());
                    model_locks.insert(token_id.clone(), *auction_id);
                }
            }

            // Invariant: A token may only appear in bids for one auction globally.
            let mut globally_seen = HashSet::new();
            for aid in 1_u64..=3_u64 {
                let auction_resp: AuctionResponse = from_json(
                    query(
                        deps.as_ref(),
                        mock_env_at(1600),
                        QueryMsg::GetAuction { auction_id: aid },
                    )
                    .unwrap(),
                )
                .unwrap();

                for bid in &auction_resp.bids {
                    for tid in &bid.token_ids {
                        prop_assert!(globally_seen.insert(tid.clone()));
                    }
                }
            }
        }

        // Finalize all auctions and drain loser escrow.
        for aid in 1_u64..=3_u64 {
            let finalize = execute(
                deps.as_mut(),
                mock_env_at(4000),
                mock_info(ADMIN, &[]),
                ExecuteMsg::FinalizeAuction { auction_id: aid },
            );
            prop_assert!(finalize.is_ok());
        }

        for aid in 1_u64..=3_u64 {
            for bidder in &BIDDERS {
                let _ = execute(
                    deps.as_mut(),
                    mock_env_at(4100),
                    mock_info(bidder, &[]),
                    ExecuteMsg::WithdrawBid { auction_id: aid },
                );
            }
        }

        // Invariant: Once all auctions are finalized and withdrawals are done,
        // there must be no lingering global bid-token locks.
        for tid in model_locks.keys() {
            prop_assert!(!crate::state::ESCROWED_BID_TOKENS.has(
                deps.as_ref().storage,
                tid.as_str(),
            ));
        }
    }
}
