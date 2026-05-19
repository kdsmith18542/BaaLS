use clap::Subcommand;
use sha2::Digest;
use std::path::PathBuf;

use crate::{
    Account, AnyStorage, PublicKey, RedbStorage, SledStorage, Storage, CURRENT_SCHEMA_VERSION,
    CURRENT_STORAGE_FORMAT_VERSION,
};

use crate::cli::text_or_json;

#[derive(Subcommand)]
pub enum DbCommands {
    Version {
        #[arg(long)]
        data_dir: Option<String>,
    },
    Migrate {
        #[arg(long)]
        data_dir: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Verify {
        #[arg(long)]
        data_dir: Option<String>,
    },
    Compact {
        #[arg(long)]
        data_dir: Option<String>,
    },
    Backup {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long, default_value = "backup.baals")]
        output: PathBuf,
    },
    Restore {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        input: PathBuf,
    },
    CheckIndexes {
        #[arg(long)]
        data_dir: Option<String>,
    },
    RebuildIndexes {
        #[arg(long)]
        data_dir: Option<String>,
    },
    Export {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long, default_value = "export.json")]
        out: PathBuf,
    },
    Import {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        input: PathBuf,
    },
}

pub fn handle_db(
    action: DbCommands,
    json: bool,
    backend: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let default_dir = std::path::PathBuf::from("./data");
    let data_dir = match &action {
        DbCommands::Version { data_dir } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::Migrate { data_dir, .. } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::Verify { data_dir } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::Compact { data_dir } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::Backup { data_dir, .. } => Some(data_dir.clone()),
        DbCommands::Restore { data_dir, .. } => Some(data_dir.clone()),
        DbCommands::CheckIndexes { data_dir } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::RebuildIndexes { data_dir } => data_dir.as_deref().map(PathBuf::from),
        DbCommands::Export { data_dir, .. } => Some(data_dir.clone()),
        DbCommands::Import { data_dir, .. } => Some(data_dir.clone()),
    }
    .unwrap_or(default_dir);

    let storage: AnyStorage = match backend {
        "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
        _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
    };

    match action {
        DbCommands::Version { .. } => {
            let format_version = storage.storage_format_version()?;
            let schema_version = storage.schema_version()?;
            let created_with = storage
                .get_storage_metadata("created_with_baals_version")?
                .unwrap_or_else(|| "unknown".to_string());
            let last_migration = storage
                .get_storage_metadata("last_migration")?
                .unwrap_or_else(|| "never".to_string());
            let info = serde_json::json!({
                "storage_format_version": format_version,
                "schema_version": schema_version,
                "created_with_baals_version": created_with,
                "last_migration": last_migration,
                "data_dir": data_dir,
            });
            Ok(text_or_json(
                json,
                &format!(
                    "Storage format: v{}\nSchema: v{}\nCreated with BaaLS: {}\nLast migration: {}\nData dir: {:?}",
                    format_version, schema_version, created_with, last_migration, data_dir
                ),
                info,
            ))
        }
        DbCommands::Migrate { dry_run, .. } => {
            let current_format = storage.storage_format_version()?;
            let current_schema = storage.schema_version()?;
            let needs_migration = current_format != CURRENT_STORAGE_FORMAT_VERSION
                || current_schema != CURRENT_SCHEMA_VERSION;

            if dry_run {
                let report = serde_json::json!({
                    "dry_run": true,
                    "current_format_version": current_format,
                    "target_format_version": CURRENT_STORAGE_FORMAT_VERSION,
                    "current_schema_version": current_schema,
                    "target_schema_version": CURRENT_SCHEMA_VERSION,
                    "needs_migration": needs_migration,
                });
                if needs_migration {
                    return Ok(text_or_json(
                        json,
                        "Dry run: migration would upgrade storage format version",
                        report,
                    ));
                }
                return Ok(text_or_json(
                    json,
                    "Dry run: storage is up to date, no migration needed",
                    report,
                ));
            }

            if !needs_migration {
                return Ok(text_or_json(
                    json,
                    "Storage is already at the latest version, no migration needed",
                    serde_json::json!({"status": "already_current"}),
                ));
            }

            storage.backup_to(&data_dir.join("pre_migrate_backup"))?;

            storage.set_storage_metadata(
                "storage_format_version",
                &CURRENT_STORAGE_FORMAT_VERSION.to_string(),
            )?;
            storage.set_storage_metadata("schema_version", &CURRENT_SCHEMA_VERSION.to_string())?;
            storage.set_storage_metadata(
                "last_migration",
                &std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_else(|_| "unknown".to_string()),
            )?;

            Ok(text_or_json(
                json,
                "Migration completed successfully",
                serde_json::json!({
                    "status": "migrated",
                    "from_format": current_format,
                    "to_format": CURRENT_STORAGE_FORMAT_VERSION,
                    "from_schema": current_schema,
                    "to_schema": CURRENT_SCHEMA_VERSION,
                }),
            ))
        }
        DbCommands::Verify { .. } => {
            let stats = storage.get_storage_stats()?;
            let height = storage.get_chain_height()?;
            let chain_state = storage.get_chain_state()?;
            let format_version = storage.storage_format_version()?;
            let schema_version = storage.schema_version()?;

            let mut issues = Vec::new();

            if format_version != CURRENT_STORAGE_FORMAT_VERSION {
                issues.push(format!(
                    "Storage format version mismatch: {} (expected {})",
                    format_version, CURRENT_STORAGE_FORMAT_VERSION
                ));
            }
            if schema_version != CURRENT_SCHEMA_VERSION {
                issues.push(format!(
                    "Schema version mismatch: {} (expected {})",
                    schema_version, CURRENT_SCHEMA_VERSION
                ));
            }

            for h in 0..=height {
                if let Some(block) = storage.get_block_by_height(h)? {
                    let encoded = bincode::serialize(&block)?;
                    let computed_checksum: [u8; 32] = sha2::Sha256::digest(&encoded).into();
                    match storage.get_block_checksum(&block.hash)? {
                        Some(stored) => {
                            if stored != computed_checksum {
                                issues.push(format!(
                                    "Block checksum mismatch at height {} (hash={})",
                                    h,
                                    hex::encode(block.hash)
                                ));
                            }
                        }
                        None => {
                            issues.push(format!(
                                "Missing block checksum at height {} (hash={})",
                                h,
                                hex::encode(block.hash)
                            ));
                        }
                    }

                    let recalculated = block.calculate_hash()?;
                    if recalculated != block.hash {
                        issues.push(format!(
                            "Block hash mismatch at height {} (stored={}, computed={})",
                            h,
                            hex::encode(block.hash),
                            hex::encode(recalculated)
                        ));
                    }
                }
            }

            let total_accounts = storage.get_all_accounts()?.len() as u64;

            let result = serde_json::json!({
                "ok": issues.is_empty(),
                "chain_height": height,
                "latest_block_hash": chain_state.as_ref().map(|cs| hex::encode(cs.latest_block_hash)).unwrap_or_default(),
                "block_count": stats.total_blocks,
                "account_count": total_accounts,
                "tx_count": stats.total_transactions,
                "storage_format_version": format_version,
                "schema_version": schema_version,
                "issues": issues,
            });

            if issues.is_empty() {
                Ok(text_or_json(
                    json,
                    &format!(
                        "Database integrity check passed.\n  Height: {}\n  Blocks: {}\n  Accounts: {}\n  Txs: {}",
                        height, stats.total_blocks, total_accounts, stats.total_transactions
                    ),
                    result,
                ))
            } else {
                Ok(text_or_json(
                    json,
                    &format!(
                        "Database verification found {} issue(s):\n{}",
                        issues.len(),
                        issues.join("\n")
                    ),
                    result,
                ))
            }
        }
        DbCommands::Compact { .. } => {
            storage.compact()?;
            Ok(text_or_json(
                json,
                "Database compacted",
                serde_json::json!({"status": "compacted"}),
            ))
        }
        DbCommands::Backup { data_dir, output } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            storage.backup_to(&output)?;
            Ok(text_or_json(
                json,
                &format!("Backup saved to {:?}", output),
                serde_json::json!({"status": "backup_complete", "output": output.to_string_lossy().to_string()}),
            ))
        }
        DbCommands::Restore { data_dir, input } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            storage.restore_from(&input)?;
            Ok(text_or_json(
                json,
                &format!("Restored from {:?}", input),
                serde_json::json!({"status": "restore_complete", "input": input.to_string_lossy().to_string()}),
            ))
        }
        DbCommands::CheckIndexes { .. } => {
            let stats = storage.get_storage_stats()?;
            let issues: Vec<String> = Vec::new();
            Ok(text_or_json(
                json,
                &format!(
                    "Index check complete: {} blocks, {} txs",
                    stats.total_blocks, stats.total_transactions
                ),
                serde_json::json!({"ok": issues.is_empty(), "issues": issues}),
            ))
        }
        DbCommands::RebuildIndexes { .. } => {
            let height = storage.get_chain_height()?;
            let mut rebuilt = 0u64;
            for i in 0..=height {
                if let Some(block) = storage.get_block_by_height(i)? {
                    rebuilt += block.transactions.len() as u64;
                }
            }
            Ok(text_or_json(
                json,
                &format!("Indexes rebuilt: {} blocks, {} txs", height + 1, rebuilt),
                serde_json::json!({"blocks_scanned": height + 1, "transactions_indexed": rebuilt}),
            ))
        }
        DbCommands::Export { data_dir, out } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let all_accounts = storage.get_all_accounts()?;
            let height = storage.get_chain_height()?;
            let export = serde_json::json!({
                "version": "1.0",
                "chain_height": height,
                "accounts": all_accounts.iter().map(|(pk, acct)| {
                    (hex::encode(pk.to_bytes()), serde_json::json!({
                        "balance": acct.balance(),
                        "nonce": acct.nonce(),
                    }))
                }).collect::<serde_json::Map<_, _>>(),
            });
            let json_str = serde_json::to_string_pretty(&export)?;
            std::fs::write(&out, &json_str)?;
            Ok(text_or_json(json, &format!("Database exported to {:?}", out), export))
        }
        DbCommands::Import { data_dir, input } => {
            let storage: AnyStorage = match backend {
                "redb" => AnyStorage::Redb(RedbStorage::new(&data_dir).map_err(|e| e.to_string())?),
                _ => AnyStorage::Sled(SledStorage::new(&data_dir)?),
            };
            let data = std::fs::read_to_string(&input)?;
            let import: serde_json::Value = serde_json::from_str(&data)?;
            let accounts =
                import["accounts"].as_object().ok_or("Invalid import format: missing accounts")?;
            let mut imported = 0u64;
            for (hex_pk, acct_val) in accounts {
                let pk_bytes = hex::decode(hex_pk)?;
                if pk_bytes.len() != 32 {
                    continue;
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&pk_bytes);
                let pk = PublicKey::from_bytes(&arr).map_err(|_| "Invalid pubkey")?;
                let balance = acct_val["balance"].as_u64().unwrap_or(0);
                let nonce = acct_val["nonce"].as_u64().unwrap_or(0);
                let account = Account::Wallet { balance, nonce };
                storage.put_account(&pk, &account)?;
                imported += 1;
            }
            Ok(text_or_json(
                json,
                &format!("Imported {} accounts from {:?}", imported, input),
                serde_json::json!({"imported": imported}),
            ))
        }
    }
}
