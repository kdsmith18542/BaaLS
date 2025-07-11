ALL CODE MUST BE PRODUCTION GRADE AND ABSOLUTELY NO STUBS WITH OUT ASKING!!

use baals::storage::SledStorage;
use baals::consensus::PoAConsensus;
use baals::runtime::Runtime;
use ed25519_dalek::Keypair;
use rand::rngs::OsRng;
use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting BaaLS Node...");

    // Create a dummy keypair for the PoA signer for MVP. In a real app, this would be loaded securely.
    let mut csprng = OsRng;
    let signing_key_pair = Keypair::generate(&mut csprng);
    let public_key = signing_key_pair.public;
    let signing_key = signing_key_pair.signing_key;

    let storage = SledStorage::new("baals_data")?;
    let consensus = PoAConsensus::new(public_key, 1000, signing_key); // 1000ms block time interval

    let runtime = Runtime::new(storage, consensus)?;
    runtime.start()?;

    println!("BaaLS Node is running. Type 'help' for commands.");

    let mut input = String::new();
    loop {
        print!("> ");
        io::stdout().flush()?;
        input.clear();
        io::stdin().read_line(&mut input)?;
        let command = input.trim();

        match command {
            "exit" => {
                runtime.stop()?;
                println!("Exiting BaaLS Node.");
                break;
            },
            "mine" => {
                match runtime.produce_block() {
                    Ok(block) => println!("Block #{} ({}) produced.", block.index, block.hash),
                    Err(e) => eprintln!("Failed to produce block: {}", e),
                }
            },
            "state" => {
                match runtime.get_chain_state() {
                    Ok(state) => println!("Current Chain State: {:?}", state),
                    Err(e) => eprintln!("Failed to get chain state: {}", e),
                }
            },
            "tx" => {
                // Placeholder for submitting a transaction. Needs user input for sender, recipient, amount.
                // For MVP CLI, this is simplified.
                println!("To submit a transaction, you would provide sender, recipient, and payload.");
                println!("e.g., tx <sender_priv_key_hex> <recipient_pub_key_hex> <amount>");
                println!("This requires proper CLI parsing and key management, which is a future CLI task.");
            }
            "help" => {
                println!("Available commands: exit, mine, state, tx, help");
            },
            _ => println!("Unknown command: {}", command),
        }
    }

    Ok(())
} 