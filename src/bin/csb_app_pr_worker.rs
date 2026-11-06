//! Run only through the networkless App PR sandbox.
use std::io::Read;
fn main() {
    let result = (|| -> Result<Vec<u8>, ()> {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(());
        }
        let request = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let root = std::env::temp_dir().join(format!("csb-app-pr-{}", std::process::id()));
        let result =
            flowdiff::github_app_setup::analyze_pr_objects(&request, &root).map_err(|_| ())?;
        let output = serde_json::to_vec(&result).map_err(|_| ())?;
        if output.len() > 16 * 1024 * 1024 {
            return Err(());
        }
        Ok(output)
    })();
    match result {
        Ok(bytes) => {
            use std::io::Write;
            if std::io::stdout().write_all(&bytes).is_err() {
                std::process::exit(1);
            }
        }
        Err(()) => {
            eprintln!("Scoped PR analysis unavailable; no report published");
            std::process::exit(1);
        }
    }
}
