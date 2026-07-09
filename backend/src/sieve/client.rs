use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

/// Upload a Sieve script via ManageSieve (RFC 5804), plain TCP (no TLS).
/// Intended for internal Docker use where Rav and Dovecot share a network.
pub async fn push_script(
    host: &str,
    port: u16,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    timeout(Duration::from_secs(10), do_push(host, port, email, password, script_name, script))
        .await
        .map_err(|_| "ManageSieve: connection timed out".to_string())?
}

async fn do_push(
    host: &str,
    port: u16,
    email: &str,
    password: &str,
    script_name: &str,
    script: &str,
) -> Result<(), String> {
    let stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("ManageSieve: connect failed: {e}"))?;

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Read server greeting
    read_response(&mut reader).await?;

    // AUTHENTICATE "PLAIN" base64(\0email\0password)
    let mut creds = vec![0u8];
    creds.extend_from_slice(email.as_bytes());
    creds.push(0);
    creds.extend_from_slice(password.as_bytes());
    let encoded = base64::engine::general_purpose::STANDARD.encode(&creds);

    writer
        .write_all(format!("AUTHENTICATE \"PLAIN\" \"{encoded}\"\r\n").as_bytes())
        .await
        .map_err(|e| format!("ManageSieve: write auth failed: {e}"))?;
    read_response(&mut reader).await?;

    // PUTSCRIPT
    let script_bytes = script.as_bytes();
    let byte_count = script_bytes.len();
    writer
        .write_all(
            format!("PUTSCRIPT \"{script_name}\" {{{byte_count}+}}\r\n").as_bytes(),
        )
        .await
        .map_err(|e| format!("ManageSieve: write putscript header failed: {e}"))?;
    writer
        .write_all(script_bytes)
        .await
        .map_err(|e| format!("ManageSieve: write script body failed: {e}"))?;
    writer
        .write_all(b"\r\n")
        .await
        .map_err(|e| format!("ManageSieve: write putscript terminator failed: {e}"))?;
    read_response(&mut reader).await?;

    // SETACTIVE
    writer
        .write_all(format!("SETACTIVE \"{script_name}\"\r\n").as_bytes())
        .await
        .map_err(|e| format!("ManageSieve: write setactive failed: {e}"))?;
    read_response(&mut reader).await?;

    // LOGOUT
    let _ = writer.write_all(b"LOGOUT\r\n").await;

    Ok(())
}

/// Read response lines until an OK, NO, or BYE line is found.
async fn read_response(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Result<(), String> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("ManageSieve: read error: {e}"))?;
        if n == 0 {
            return Err("ManageSieve: connection closed unexpectedly".to_string());
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("OK") {
            return Ok(());
        }
        if trimmed.starts_with("NO") || trimmed.starts_with("BYE") {
            return Err(format!("ManageSieve: server error: {}", line.trim()));
        }
        // Capability lines and continuation lines - keep reading
    }
}
