fn main() {
    if std::env::args().nth(1).as_deref() == Some("--installation-notice") {
        labelize_printer_updater::platform::installation_notice();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("labelize-printer-updater {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let result = (|| -> anyhow::Result<()> {
        anyhow::ensure!(
            std::env::args().nth(1).as_deref() == Some("--handoff"),
            "Expected --handoff request.json"
        );
        let file = std::env::args_os()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("Missing handoff request"))?;
        let ticket = serde_json::from_slice(&std::fs::read(file)?)?;
        labelize_printer_updater::handoff::run(ticket)
    })();
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
