use solana_client::rpc_client::RpcClient;
use solana_sdk::clock::Slot;
use solana_transaction_status::{EncodedTransaction, UiTransactionEncoding, UiTransaction};
use std::collections::HashSet;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    let rpc_url = "http://127.0.0.1:8899"; // URL of the local Solana validator
    let client = RpcClient::new(rpc_url.to_string());

    let mut last_slot: Slot = 0;
    let mut seen_blocks: HashSet<Slot> = HashSet::new();

    loop {
        let current_slot = client.get_slot().unwrap();
        if current_slot > last_slot {
            for slot in (last_slot + 1)..=current_slot {
                if seen_blocks.contains(&slot) {
                    continue;
                }

                match client.get_block(slot) {
                    Ok(block) => {
                        let block_hash = block.blockhash.to_string();
                        println!("New block created! Slot: {}, Block hash: {}", slot, block_hash);
                        for transaction_with_meta in block.transactions {
                            if let EncodedTransaction::Json(transaction) = &transaction_with_meta.transaction {
                                for signature in &transaction.signatures {
                                    println!("Transaction hash: {}", signature);
                                }
                            }
                        }
                        seen_blocks.insert(slot);
                    }
                    Err(e) => {
                        if e.to_string().contains("Block") && e.to_string().contains("does not exist on node") {
                            // Extract the first available block from the error message
                            if let Some(start_index) = e.to_string().find("First available block: ") {
                                if let Some(end_index) = e.to_string()[start_index..].find(',') {
                                    if let Ok(first_available_block) = e.to_string()[start_index + 23..start_index + end_index].parse::<Slot>() {
                                        last_slot = first_available_block;
                                        println!("Adjusting to first available block: {}", first_available_block);
                                        break;
                                    }
                                }
                            }
                        } else {
                            eprintln!("Error fetching block {}: {:?}", slot, e);
                        }
                    }
                }
            }
            last_slot = current_slot;
        }
        sleep(Duration::from_secs(1)).await; // Adjust the delay as needed
    }
}
