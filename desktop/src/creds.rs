//! Saved passwords in Windows Credential Manager.
//!
//! Entries: target `TapFeed/<account>`, where account is `camera:<user>@<host>` or
//! `cloud:<user>@<host>`. Entries saved under the old name are read once and copied:
//! `Tapo-Viewer/<account>` (2.0 test builds) and, from the Python app's `keyring`,
//! `Tapo-Viewer` or `<account>@Tapo-Viewer`. Old entries are left for the old app.

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

const SERVICE: &str = "TapFeed";
const OLD_SERVICE: &str = "Tapo-Viewer";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

pub fn account(kind: &str, host: &str, user: &str) -> String {
    format!("{kind}:{}@{}", user.trim().to_lowercase(), host.trim().to_lowercase())
}

/// Read a generic credential; returns (username, password).
fn read(target: &str) -> Option<(String, String)> {
    unsafe {
        let mut cred: *mut CREDENTIALW = null_mut();
        if CredReadW(wide(target).as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
            return None;
        }
        let c = &*cred;
        let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let units: Vec<u16> = blob.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let password = String::from_utf16_lossy(&units);
        let user = from_wide(c.UserName);
        CredFree(cred as *const _);
        Some((user, password))
    }
}

fn write(target: &str, user: &str, password: &str) -> Result<(), String> {
    let mut target_w = wide(target);
    let mut user_w = wide(user);
    let mut blob: Vec<u8> = password.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let cred = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_w.as_mut_ptr(),
        Comment: null_mut(),
        LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: null_mut(),
        TargetAlias: null_mut(),
        UserName: user_w.as_mut_ptr(),
    };
    if unsafe { CredWriteW(&cred, 0) } == 0 {
        return Err(format!("Credential Manager refused to save ({})", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn delete(target: &str) {
    unsafe {
        CredDeleteW(wide(target).as_ptr(), CRED_TYPE_GENERIC, 0);
    }
}

fn target_for(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

pub fn load(account: &str) -> Option<String> {
    if let Some((_, pw)) = read(&target_for(account)) {
        return Some(pw);
    }
    // Saved under the old name: 2.0 test builds, then the Python app's keyring layout.
    let legacy = [
        format!("{OLD_SERVICE}/{account}"),
        OLD_SERVICE.to_string(),
        format!("{account}@{OLD_SERVICE}"),
    ];
    for t in legacy {
        if let Some((user, pw)) = read(&t) {
            if user == account {
                let _ = write(&target_for(account), account, &pw);
                return Some(pw);
            }
        }
    }
    None
}

pub fn save(account: &str, password: &str) -> Result<(), String> {
    write(&target_for(account), account, password)
}

/// "Forget" means forget everywhere on this PC, old names included.
pub fn remove(account: &str) {
    delete(&target_for(account));
    delete(&format!("{OLD_SERVICE}/{account}"));
    if let Some((user, _)) = read(OLD_SERVICE) {
        if user == account {
            delete(OLD_SERVICE);
        }
    }
    delete(&format!("{account}@{OLD_SERVICE}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes test entries to Credential Manager and removes them: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn reads_old_name_and_copies() {
        let acc = "camera:tapfeed-selftest@192.0.2.1";
        remove(acc);
        write(&format!("{OLD_SERVICE}/{acc}"), acc, "pässwörd 1").unwrap();
        assert_eq!(load(acc).as_deref(), Some("pässwörd 1"), "old 2.0 entry is read");
        assert!(read(&target_for(acc)).is_some(), "and copied under the new name");
        remove(acc);
        assert!(load(acc).is_none(), "forget removes both");

        write(&format!("{acc}@{OLD_SERVICE}"), acc, "py").unwrap();
        assert_eq!(load(acc).as_deref(), Some("py"), "Python keyring layout is read");
        remove(acc);
        assert!(load(acc).is_none());
    }
}
