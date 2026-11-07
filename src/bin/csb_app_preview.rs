//! Networkless worker protocol. Host uses scripts/app_preview_sandbox.py only.
use std::io::Read;
fn main() {
    let result = (|| -> Result<String, ()> {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(());
        }
        let input = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let root = std::env::temp_dir().join(format!("csb-app-preview-{}", std::process::id()));
        let result = flowdiff::github_app_setup::analyze_preview(&input, &root).map_err(|_| ())?;
        let output = serde_json::to_string(&result).map_err(|_| ())?;
        if output.len() > 1024 * 1024 {
            return Err(());
        }
        Ok(output)
    })();
    match result {
        Ok(output) => println!("{output}"),
        Err(()) => {
            eprintln!("App preview failed; no architecture accepted");
            std::process::exit(1)
        }
    }
}
