//! stdio entry point for the SysML v2 language server.

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    // not `lsp_server::Connection::stdio`, which ends the conversation at
    // the first message it cannot read: see `sysml_lsp::wire`
    let (connection, io_threads) = sysml_lsp::wire::stdio();
    sysml_lsp::run(&connection)?;
    // the writer thread only terminates once the connection (and with it the
    // outgoing channel) is dropped
    drop(connection);
    io_threads.join()?;
    Ok(())
}
