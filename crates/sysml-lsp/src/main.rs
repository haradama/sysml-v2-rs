//! stdio entry point for the SysML v2 language server.

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    sysml_lsp::run(&connection)?;
    // the writer thread only terminates once the connection (and with it the
    // outgoing channel) is dropped
    drop(connection);
    io_threads.join()?;
    Ok(())
}
