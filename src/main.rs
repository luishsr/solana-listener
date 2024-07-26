use bellman::{groth16, Circuit, ConstraintSystem, SynthesisError};
use blstrs::{Bls12, Scalar as Fr};
use ff::{Field, PrimeField};
use rand::thread_rng;
use serde::{Serialize, Deserialize};
use sha2::{Digest, Sha256};
use solana_client::rpc_client::RpcClient;
use solana_sdk::clock::Slot;
use solana_transaction_status::{EncodedTransaction};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tokio::time::{sleep, Duration};

#[derive(Serialize, Deserialize)]
struct TransactionProof {
    transaction_hash: String,
    proof: String,
}

#[derive(Serialize, Deserialize)]
struct BlockProof {
    slot: Slot,
    block_hash: String,
    transactions: Vec<TransactionProof>,
}

// Define the circuit for block validation
struct BlockCircuit {
    pub block_hash: Option<Fr>,
    pub transaction_hashes: Vec<Option<Fr>>,
}

impl Circuit<Fr> for BlockCircuit {
    fn synthesize<CS: ConstraintSystem<Fr>>(self, cs: &mut CS) -> Result<(), SynthesisError> {
        // Allocate the block hash
        let block_hash_var = cs.alloc(
            || "block hash",
            || self.block_hash.ok_or(SynthesisError::AssignmentMissing),
        )?;

        // Hash the transaction hashes
        let mut hasher = Sha256::new();
        for tx_hash in self.transaction_hashes.iter() {
            if let Some(hash) = tx_hash {
                hasher.update(hash.to_repr());
            }
        }

        // Convert the final hash to a field element
        let result_hash = hasher.finalize();
        let mut result_hash_bytes = [0u8; 32];
        result_hash_bytes.copy_from_slice(&result_hash);
        let result_hash_fr = Fr::from_repr(result_hash_bytes).unwrap_or_else(||Fr::ZERO);

        // Constrain the computed hash to be equal to the given block hash
        cs.enforce(
            || "block hash constraint",
            |lc| lc + block_hash_var,
            |lc| lc + CS::one(),
            |lc| lc + (result_hash_fr, CS::one()),
        );

        Ok(())
    }
}

// Function to generate a proof for a block
fn generate_block_proof(block_hash: Fr, transaction_hashes: Vec<Fr>) -> String {
    // Create an instance of the circuit with the block data
    let circuit = BlockCircuit {
        block_hash: Some(block_hash),
        transaction_hashes: transaction_hashes.iter().map(|&x| Some(x)).collect(),
    };

    // Generate parameters
    let rng = &mut thread_rng();
    let params = {
        let empty_circuit = BlockCircuit {
            block_hash: None,
            transaction_hashes: vec![None; transaction_hashes.len()],
        };
        groth16::generate_random_parameters::<Bls12, _, _>(empty_circuit, rng).unwrap()
    };

    // Create a proof
    let proof = groth16::create_random_proof(circuit, &params, rng).unwrap();

    // Serialize the proof
    format!("{:?}", proof)
}

fn str_to_fr(data: &str) -> Option<Fr> {
    // Convert string to bytes and then to Fr (handling errors)
    let hash = Sha256::digest(data.as_bytes());
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(&hash);
    println!("Converting hash to field element: {:?}", hash_bytes);
    Some(Fr::from_repr(hash_bytes).unwrap_or_else(||Fr::ZERO))
}

#[tokio::main]
async fn main() {
    let rpc_url = "http://127.0.0.1:8899"; // URL of the local Solana validator
    let client = RpcClient::new(rpc_url.to_string());

    // Create and clean the proofs directory
    let proofs_dir = Path::new("proofs");
    if proofs_dir.exists() {
        fs::remove_dir_all(proofs_dir).expect("Unable to clean proofs directory");
    }
    fs::create_dir(proofs_dir).expect("Unable to create proofs directory");

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
                        let block_hash_str = block.blockhash.to_string();
                        println!("New block created! Slot: {}, Block hash: {}", slot, block_hash_str);

                        if let Some(block_hash) = str_to_fr(&block_hash_str) {
                            let mut block_proof = BlockProof {
                                slot,
                                block_hash: block_hash_str.clone(),
                                transactions: Vec::new(),
                            };

                            let mut transaction_hashes = vec![];

                            for transaction_with_meta in block.transactions {
                                if let EncodedTransaction::Json(transaction) = &transaction_with_meta.transaction {
                                    for signature in &transaction.signatures {
                                        let transaction_hash_str = signature.to_string();
                                        println!("Transaction hash: {}", transaction_hash_str);

                                        if let Some(transaction_hash) = str_to_fr(&transaction_hash_str) {
                                            transaction_hashes.push(transaction_hash);

                                            // Generate ZKP proof for the transaction (dummy example)
                                            let proof = generate_block_proof(transaction_hash, transaction_hashes.clone());

                                            // Add transaction proof to block proof
                                            block_proof.transactions.push(TransactionProof {
                                                transaction_hash: transaction_hash_str,
                                                proof,
                                            });
                                        } else {
                                            println!("Error converting transaction hash to field element: {}", transaction_hash_str);
                                        }
                                    }
                                }
                            }

                            // Generate block proof
                            let block_proof_str = generate_block_proof(block_hash, transaction_hashes);

                            // Save the block proof to a JSON file
                            save_proof_to_json(&block_proof, slot, &proofs_dir);

                            seen_blocks.insert(slot);
                        } else {
                            println!("Error converting block hash to field element: {}", block_hash_str);
                        }
                    }
                    Err(e) => {
                        let error_message = e.to_string();
                        if error_message.contains("Slot was skipped") || error_message.contains("Block cleaned up") {
                            if let Some(start_index) = error_message.find("First available block: ") {
                                if let Some(end_index) = error_message[start_index..].find(',') {
                                    if let Ok(first_available_block) = error_message[start_index + 23..start_index + end_index].parse::<Slot>() {
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

fn save_proof_to_json(block_proof: &BlockProof, slot: Slot, proofs_dir: &Path) {
    let file_name = proofs_dir.join(format!("block_proof_{}.json", slot));
    let mut file = File::create(&file_name).expect("Unable to create file");
    let json_data = serde_json::to_string_pretty(&block_proof).expect("Unable to serialize proof");

    file.write_all(json_data.as_bytes()).expect("Unable to write data to file");

    println!("Saved block proof to {:?}", file_name);
}
