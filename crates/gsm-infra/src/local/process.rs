use gsm_domain::{ProcessState, local::LocalServer};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub pid: u32,
    pub created: u64,
    pub executable: PathBuf,
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn opened(pid: u32) -> Result<Handle, String> {
        open_with_access(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    }
    fn open_with_access(pid: u32, access: u32) -> Result<Handle, String> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        if h.is_null() {
            Err(format!(
                "OpenProcess に失敗 [PID: {pid}]: {}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(Handle(h))
        }
    }
    pub fn identity(pid: u32) -> Result<Identity, String> {
        let h = opened(pid)?;
        identity_handle(pid, &h)
    }
    fn identity_handle(pid: u32, h: &Handle) -> Result<Identity, String> {
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        unsafe {
            if GetProcessTimes(h.0, &mut created, &mut exit, &mut kernel, &mut user) == 0 {
                return Err(format!(
                    "GetProcessTimes に失敗 [PID: {pid}]: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if QueryFullProcessImageNameW(h.0, 0, buf.as_mut_ptr(), &mut len) == 0 {
                return Err(format!(
                    "QueryFullProcessImageNameW に失敗 [PID: {pid}]: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(Identity {
            pid,
            created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            executable: PathBuf::from(
                String::from_utf16(&buf[..len as usize])
                    .map_err(|_| "プロセスパスの文字列変換に失敗")?,
            ),
        })
    }
    /// Acquire and verify before sending shutdown. A retained handle remains waitable
    /// during teardown, when reopening or querying the process can fail with access denied.
    pub struct ExitWaiter(Handle);
    impl ExitWaiter {
        pub fn open(expected: &Identity) -> Result<Self, String> {
            let handle = open_with_access(
                expected.pid,
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            )?;
            let actual = identity_handle(expected.pid, &handle)?;
            if actual.created != expected.created
                || super::super::paths::key(&actual.executable)
                    != super::super::paths::key(&expected.executable)
            {
                return Err("終了待機対象の PID・作成時刻・実行ファイルが一致しません".into());
            }
            Ok(Self(handle))
        }
        pub fn wait(&self, timeout: std::time::Duration) -> Result<bool, String> {
            // Never turn an oversized finite duration into Windows INFINITE (u32::MAX).
            let milliseconds = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
            match unsafe { WaitForSingleObject(self.0.0, milliseconds) } {
                WAIT_OBJECT_0 => Ok(true),
                WAIT_TIMEOUT => Ok(false),
                _ => Err(format!(
                    "WaitForSingleObject に失敗: {}",
                    std::io::Error::last_os_error()
                )),
            }
        }
    }
    pub fn candidates(spec: &LocalServer) -> Result<Vec<u32>, String> {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let snap = Handle(snap);
        let mut item = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = unsafe { Process32FirstW(snap.0, &mut item) };
        let mut out = vec![];
        let expected = spec
            .executable
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("実行ファイル名が不正")?;
        while more != 0 {
            let len = item
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(item.szExeFile.len());
            let name = String::from_utf16_lossy(&item.szExeFile[..len]);
            if name.eq_ignore_ascii_case(expected) {
                out.push(item.th32ProcessID);
            }
            more = unsafe { Process32NextW(snap.0, &mut item) };
        }
        Ok(out)
    }
    pub fn signal(pid: u32, created: u64) -> Result<(), String> {
        use windows_sys::Win32::System::Console::{
            AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
            SetConsoleCtrlHandler,
        };
        // Holding a handle across identity verification and attachment prevents PID reuse.
        let handle = opened(pid)?;
        if identity_handle(pid, &handle)?.created != created {
            return Err("プロセスの作成時刻が一致しません".into());
        }
        unsafe {
            FreeConsole();
            if AttachConsole(pid) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if SetConsoleCtrlHandler(None, 1) == 0 {
                FreeConsole();
                return Err("制御ハンドラーを登録できません".into());
            }
            let ok = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            std::thread::sleep(std::time::Duration::from_millis(200));
            FreeConsole();
            if ok == 0 {
                return Err("停止イベントを送信できません".into());
            }
        }
        Ok(())
    }
}
#[cfg(windows)]
pub use platform::{ExitWaiter, candidates, identity, signal};
#[cfg(not(windows))]
pub struct ExitWaiter;
#[cfg(not(windows))]
impl ExitWaiter {
    pub fn open(_expected: &Identity) -> Result<Self, String> {
        Err("実プロセス制御は Windows 専用です".into())
    }
    pub fn wait(&self, _timeout: std::time::Duration) -> Result<bool, String> {
        Err("実プロセス制御は Windows 専用です".into())
    }
}
#[cfg(not(windows))]
pub fn identity(_pid: u32) -> Result<Identity, String> {
    Err("実プロセス制御は Windows 専用です".into())
}
#[cfg(not(windows))]
pub fn candidates(_spec: &LocalServer) -> Result<Vec<u32>, String> {
    Err("実プロセス制御は Windows 専用です".into())
}
#[cfg(not(windows))]
pub fn signal(_pid: u32, _created: u64) -> Result<(), String> {
    Err("停止 helper は Windows 専用です".into())
}

pub fn observe(spec: &LocalServer, expected: Option<&Identity>) -> Result<ProcessState, String> {
    let ids = candidates(spec)?;
    if let Some(expected) = expected
        && ids.contains(&expected.pid)
    {
        let actual = identity(expected.pid)?;
        if actual.created == expected.created
            && super::paths::key(&actual.executable) == super::paths::key(&spec.executable)
        {
            return Ok(ProcessState::Alive);
        }
        return Ok(ProcessState::Unknown);
    }
    // A matching executable started outside this manager is never silently adopted.
    for pid in ids {
        let actual = identity(pid)?;
        if super::paths::key(&actual.executable) == super::paths::key(&spec.executable) {
            return Ok(ProcessState::Unknown);
        }
    }
    Ok(ProcessState::Absent)
}
pub fn start(spec: &LocalServer) -> Result<Identity, String> {
    if !cfg!(windows) {
        return Err("実プロセス制御は Windows 専用です".into());
    }
    let mut command = std::process::Command::new(&spec.executable);
    command
        .args(&spec.arguments)
        .current_dir(&spec.working_directory)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x10);
    } // CREATE_NEW_CONSOLE: helper must never signal the GUI console.
    let mut child = command.spawn().map_err(|e| {
        format!(
            "サーバーを起動できません [実行ファイル: {}, 作業フォルダー: {}]: {e}",
            spec.executable.display(),
            spec.working_directory.display()
        )
    })?;
    let result = identity(child.id()).map_err(|e| {
        format!(
            "起動したサーバーの識別情報を取得できません [PID: {}, 実行ファイル: {}]: {e}",
            child.id(),
            spec.executable.display()
        )
    });
    // Keep the process handle alive until it exits, while the GUI stays running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    result
}
