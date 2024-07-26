use solana_client::rpc_client::RpcClient;
use solana_sdk::clock::Slot;
use solana_transaction_status::{EncodedTransaction};
use std::collections::HashSet;
use tokio::time::{sleep, Duration};
use serde::{Serialize, Deserialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use bellman::{groth16, Circuit, ConstraintSystem, SynthesisError};
use blstrs::{Bls12, Scalar as Fr};
use ff::{Field, PrimeField};
use rand::thread_rng;

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

// A dummy circuit for demonstration. Replace with your actual circuit.
struct DummyCircuit {
    pub value: Option<Fr>,
}

impl Circuit<Fr> for DummyCircuit {
    fn synthesize<CS: ConstraintSystem<Fr>>(self, cs: &mut CS) -> Result<(), SynthesisError> {
        let value = cs.alloc(|| "value", || self.value.ok_or(SynthesisError::AssignmentMissing))?;
        cs.enforce(|| "constraint", |lc| lc + value, |lc| lc + value, |lc| lc + value);
        Ok(())
    }
}

fn generate_proof(data: &str) -> String {
    // Convert data to a field element (dummy example).
    let value = Fr::from_str_vartime(data).unwrap_or_else(|| Fr::ZERO);

    // Create an instance of the circuit with the value.
    let circuit = DummyCircuit { value: Some(value) };

    // Generate parameters.
    let rng = &mut thread_rng();
    let params = {
        let c = DummyCircuit { value: None };
        groth16::generate_random_parameters::<Bls12, _, _>(c, rng).unwrap()
    };

    // Create a proof.
    let proof = groth16::create_random_proof(circuit, &params, rng).unwrap();

    // Serialize the proof (dummy example).
    format!("{:?}", proof)
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
                        let block_hash = block.blockhash.to_string();
                        println!("New block created! Slot: {}, Block hash: {}", slot, block_hash);

                        let mut block_proof = BlockProof {
                            slot,
                            block_hash: block_hash.clone(),
                            transactions: Vec::new(),
                        };

                        for transaction_with_meta in block.transactions {
                            if let EncodedTransaction::Json(transaction) = &transaction_with_meta.transaction {
                                for signature in &transaction.signatures {
                                    let transaction_hash = signature.to_string();
                                    println!("Transaction hash: {}", transaction_hash);

                                    // Generate ZKP proof for the transaction
                                    let proof = generate_proof(&transaction_hash);

                                    // Add transaction proof to block proof
                                    block_proof.transactions.push(TransactionProof {
                                        transaction_hash,
                                        proof,
                                    });
                                }
                            }
                        }

                        // Save the block proof to a JSON file
                        save_proof_to_json(&block_proof, slot, &proofs_dir);

                        seen_blocks.insert(slot);
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
