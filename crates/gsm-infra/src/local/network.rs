use gsm_domain::local::Shutdown;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

fn packet(stream: &mut TcpStream, id: i32, kind: i32, text: &str) -> Result<(), String> {
    let size = 10 + text.len();
    if size > 65536 {
        return Err("RCON のメッセージが長すぎます".into());
    }
    let mut data = vec![];
    data.extend((size as i32).to_le_bytes());
    data.extend(id.to_le_bytes());
    data.extend(kind.to_le_bytes());
    data.extend(text.as_bytes());
    data.extend([0, 0]);
    stream
        .write_all(&data)
        .map_err(|_| "RCON の送信に失敗".into())
}
fn response(stream: &mut TcpStream) -> Result<(i32, i32), String> {
    let mut length = [0; 4];
    stream
        .read_exact(&mut length)
        .map_err(|_| "RCON の応答を受信できません")?;
    let n = i32::from_le_bytes(length);
    if !(10..=65536).contains(&n) {
        return Err("RCON の応答長が不正".into());
    }
    let mut data = vec![0; n as usize];
    stream
        .read_exact(&mut data)
        .map_err(|_| "RCON の応答が不完全")?;
    if !data.ends_with(&[0, 0]) {
        return Err("RCON の応答終端が不正".into());
    }
    Ok((
        i32::from_le_bytes(data[0..4].try_into().unwrap()),
        i32::from_le_bytes(data[4..8].try_into().unwrap()),
    ))
}
pub fn rcon(port: u16, password: &str, commands: &[String]) -> Result<(), String> {
    if password.is_empty() {
        return Err("RCON パスワードがありません".into());
    }
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(5),
    )
    .map_err(|_| "ローカル RCON に接続できません")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "RCON タイムアウト設定に失敗")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "RCON タイムアウト設定に失敗")?;
    packet(&mut stream, 101, 3, password)?;
    let mut authenticated = false;
    for _ in 0..2 {
        let (id, kind) = response(&mut stream)?;
        if id == -1 {
            return Err("RCON 認証に失敗しました".into());
        }
        if id == 101 && kind == 2 {
            authenticated = true;
            break;
        }
    }
    if !authenticated {
        return Err("RCON 認証応答が不正です".into());
    }
    for (index, command) in commands.iter().enumerate() {
        packet(&mut stream, 200 + index as i32, 2, command)?;
        if index + 1 < commands.len() {
            response(&mut stream)?;
        }
    }
    Ok(())
}
fn call(
    client: &reqwest::blocking::Client,
    port: u16,
    token: &str,
    function: &str,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut request = client
        .post(format!("https://127.0.0.1:{port}/api/v1/"))
        .json(&serde_json::json!({"function":function,"data":data}));
    if !token.is_empty() {
        request = request.bearer_auth(token)
    }
    let response = request
        .send()
        .map_err(|_| "ローカル HTTPS API に接続できません")?;
    if !response.status().is_success() {
        return Err(format!("API HTTP エラー: {}", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .take(1048577)
        .read_to_end(&mut bytes)
        .map_err(|_| "API 応答を読み取れません")?;
    if bytes.len() > 1048576 {
        return Err("API 応答が大きすぎます".into());
    }
    if bytes.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "API 応答が JSON ではありません")?;
    if value.get("errorCode").is_some() {
        return Err("API が操作を拒否しました（認証・権限を確認してください）".into());
    }
    Ok(value.get("data").cloned().unwrap_or(value))
}
pub fn api_shutdown(stop: &Shutdown) -> Result<(), String> {
    let Shutdown::Https {
        port,
        token,
        password,
    } = stop
    else {
        return Err("API 設定がありません".into());
    };
    // Certificate exception is constrained to loopback; proxies and redirects are disabled.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "API クライアントを作成できません")?;
    let mut token = token.clone();
    if token.is_empty() {
        if password.is_empty() {
            return Err("管理者パスワードまたは API トークンが必要です".into());
        }
        let data = call(
            &client,
            *port,
            "",
            "PasswordLogin",
            serde_json::json!({"MinimumPrivilegeLevel":"Administrator","Password":password}),
        )?;
        token = data
            .get("AuthenticationToken")
            .or_else(|| data.get("authenticationToken"))
            .and_then(|v| v.as_str())
            .ok_or("API トークンを取得できません")?
            .into();
    }
    call(&client, *port, &token, "Shutdown", serde_json::json!({}))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authentication_failure_never_sends_shutdown() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(response(&mut stream).unwrap(), (101, 3));
            packet(&mut stream, -1, 2, "").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let error = rcon(port, "test-password", &["shutdown".into()]).unwrap_err();
        assert!(!error.contains("test-password"));
        worker.join().unwrap();
    }
    #[test]
    fn rcon_authenticates_before_ordered_commands() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            assert_eq!(response(&mut s).unwrap(), (101, 3));
            packet(&mut s, 101, 0, "").unwrap();
            packet(&mut s, 101, 2, "").unwrap();
            assert_eq!(response(&mut s).unwrap(), (200, 2));
            packet(&mut s, 200, 0, "saved").unwrap();
            assert_eq!(response(&mut s).unwrap(), (201, 2));
        });
        rcon(
            port,
            "test-password",
            &["SaveWorld".into(), "DoExit".into()],
        )
        .unwrap();
        worker.join().unwrap();
    }
}
