use std::{error::Error, fs, path::PathBuf};

use kbc_protocol::*;
use ts_rs::{Config, TS};

fn main() -> Result<(), Box<dyn Error>> {
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/line/src/protocol/generated");
    let config = Config::new()
        .with_out_dir(&output)
        .with_import_extension(Some("js"));
    fs::create_dir_all(&output)?;
    CoreConfig::export_all(&config)?;
    ReceivedBatch::export_all(&config)?;
    CoreAction::export_all(&config)?;
    ActionResult::export_all(&config)?;
    BatchReceipt::export_all(&config)?;
    CoreStats::export_all(&config)?;
    RuntimeStatus::export_all(&config)?;
    PendingLog::export_all(&config)?;
    fs::write(
        output.join("version.ts"),
        format!("export const PROTOCOL_VERSION = {PROTOCOL_VERSION} as const;\n"),
    )?;
    Ok(())
}
