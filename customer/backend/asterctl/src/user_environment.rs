use std::io;

pub use aster_codex_config::API_KEY_ENVIRONMENT_VARIABLE as API_KEY_NAME;

#[cfg(windows)]
pub fn read_api_key() -> Option<String> {
    windows_registry::CURRENT_USER
        .open("Environment")
        .and_then(|key| key.get_string(API_KEY_NAME))
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(not(windows))]
pub fn read_api_key() -> Option<String> {
    std::env::var(API_KEY_NAME)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(windows)]
pub fn write_api_key(value: &str) -> io::Result<()> {
    let environment = windows_registry::CURRENT_USER
        .create("Environment")
        .map_err(|error| io::Error::other(error.to_string()))?;
    environment
        .set_string(API_KEY_NAME, value)
        .map_err(|error| io::Error::other(error.to_string()))?;
    broadcast_environment_change()
}

#[cfg(not(windows))]
pub fn write_api_key(_value: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "setting a persistent user API key is currently supported on Windows only",
    ))
}

#[cfg(windows)]
pub fn restore_api_key(previous: Option<&str>) -> io::Result<()> {
    let environment = windows_registry::CURRENT_USER
        .create("Environment")
        .map_err(|error| io::Error::other(error.to_string()))?;
    if let Some(value) = previous {
        environment
            .set_string(API_KEY_NAME, value)
            .map_err(|error| io::Error::other(error.to_string()))?;
    } else {
        match environment.remove_value(API_KEY_NAME) {
            Ok(()) => {}
            Err(error) if error.code().0 as u32 == 0x8007_0002 => {}
            Err(error) => return Err(io::Error::other(error.to_string())),
        }
    }
    broadcast_environment_change()
}

#[cfg(not(windows))]
pub fn restore_api_key(_previous: Option<&str>) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn broadcast_environment_change() -> io::Result<()> {
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001a;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    #[link(name = "user32")]
    unsafe extern "system" {
        #[link_name = "SendMessageTimeoutW"]
        fn send_message_timeout_w(
            window: isize,
            message: u32,
            word_parameter: usize,
            long_parameter: isize,
            flags: u32,
            timeout_ms: u32,
            result: *mut usize,
        ) -> isize;
    }

    let environment = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut message_result = 0_usize;
    // SAFETY: the UTF-16 buffer is NUL-terminated and remains alive for the synchronous call;
    // the result pointer refers to a valid local usize. The other values are documented Win32
    // constants, and no handle ownership is transferred.
    let delivered = unsafe {
        send_message_timeout_w(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1_000,
            &mut message_result,
        )
    };
    if delivered == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error().is_some_and(|code| code != 0) {
            Err(error)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}
