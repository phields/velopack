use crate::shared::{self, runtime_arch::RuntimeArch};
use anyhow::{anyhow, Result};
use normpath::PathExt;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command as Process,
    time::Duration,
};
use wait_timeout::ChildExt;
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{self, GetLastError},
        System::Threading::CreateMutexW,
    },
};
use winsafe::{self as w, co};

pub fn run_hook(app: &shared::bundle::Manifest, root_path: &PathBuf, hook_name: &str, timeout_secs: u64) -> bool {
    let sw = simple_stopwatch::Stopwatch::start_new();
    let current_path = app.get_current_path(&root_path);
    let main_exe_path = app.get_main_exe_path(&root_path);
    let ver_string = app.version.to_string();
    let args = vec![hook_name, &ver_string];
    let mut success = false;

    info!("Running {} hook...", hook_name);
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let cmd = Process::new(&main_exe_path).args(args).current_dir(&current_path).creation_flags(CREATE_NO_WINDOW).spawn();

    if let Err(e) = cmd {
        warn!("Failed to start hook {}: {}", hook_name, e);
        return false;
    }

    let mut cmd = cmd.unwrap();
    let _ = unsafe { AllowSetForegroundWindow(cmd.id()) };

    match cmd.wait_timeout(Duration::from_secs(timeout_secs)) {
        Ok(Some(status)) => {
            if status.success() {
                info!("Hook executed successfully (took {}ms)", sw.ms());
                success = true;
            } else {
                warn!("Hook exited with non-zero exit code: {}", status.code().unwrap_or(0));
            }
        }
        Ok(None) => {
            let _ = cmd.kill();
            error!("Process timed out after {}s", timeout_secs);
        }
        Err(e) => {
            error!("Error waiting for process to finish: {}", e);
        }
    }

    // in case the hook left running processes
    let _ = shared::force_stop_package(&root_path);
    success
}

pub struct MutexDropGuard {
    mutex: Foundation::HANDLE,
}

impl Drop for MutexDropGuard {
    fn drop(&mut self) {
        unsafe {
            Foundation::CloseHandle(self.mutex).ok();
        }
    }
}

pub fn create_global_mutex(app: &shared::bundle::Manifest) -> Result<MutexDropGuard> {
    let mutex_name = format!("velopack-{}", &app.id);
    info!("Attempting to open global system mutex: '{}'", &mutex_name);
    let encoded = mutex_name.encode_utf16().chain([0u16]).collect::<Vec<u16>>();
    let pw = PCWSTR(encoded.as_ptr());
    let mutex = unsafe { CreateMutexW(None, true, pw) }?;
    match unsafe { GetLastError() } {
        Foundation::ERROR_SUCCESS => Ok(MutexDropGuard { mutex }),
        Foundation::ERROR_ALREADY_EXISTS => Err(anyhow!("Another installer or updater for this application is running, quit that process and try again.")),
        err => Err(anyhow!("Unable to create global mutex. Error code {:?}", err)),
    }
}

pub fn is_sub_path<P1: AsRef<Path>, P2: AsRef<Path>>(path: P1, parent: P2) -> Result<bool> {
    let path = path.as_ref().to_string_lossy().to_lowercase();
    let parent = parent.as_ref().to_string_lossy().to_lowercase();
    let parent = parent.trim_end_matches('\\').trim_end_matches('/').to_owned() + "\\";

    // some quick bails before we do the more expensive path normalization
    if path.is_empty() || parent.is_empty() {
        return Ok(false);
    }

    if path.len() < parent.len() {
        return Ok(false);
    }

    if path.starts_with(&parent) {
        return Ok(true);
    }

    let path = w::ExpandEnvironmentStrings(&path)?;
    let parent = w::ExpandEnvironmentStrings(&parent)?;

    let path = Path::new(&path);
    let parent = Path::new(&parent);

    // we just bail if paths are not absolute. in the cases where we use this function,
    // we should have absolute paths from the file system (e.g. iterating running processes, reading shortcuts)
    // if we receive a relative path, it's likely coming from a shortcut target/working directory
    // that we can't resolve with ExpandEnvironmentStrings
    if !path.is_absolute() || !parent.is_absolute() {
        return Ok(false);
    }

    // calls GetFullPathNameW
    let path = path.normalize_virtually()?.as_path().to_string_lossy().to_lowercase();
    let parent = parent.normalize_virtually()?.as_path().to_string_lossy().to_lowercase();

    let path = PathBuf::from(path);
    let parent = PathBuf::from(parent);

    // use path.starts_with instead of string.starts_with because it compares by path component
    Ok(path.starts_with(parent))
}

#[test]
fn test_is_sub_path_works_with_existing_paths() {
    let path = PathBuf::from(r"C:\Windows\System32/dxdiag.exe");
    let parent = PathBuf::from(r"c:\windows/system32\");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Windows\System32/dxdiag.exe");
    let parent = PathBuf::from(r"c:\windows/");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Windows\System32/dxdiag.exe");
    let parent = PathBuf::from(r"c:\windows\");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Windows\System32/dxdiag.exe");
    let parent = PathBuf::from(r"c:\windows");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Windows\System32/dxdiag.exe");
    let parent = PathBuf::from(r"c:/");
    assert!(is_sub_path(&path, &parent).unwrap());
}

#[test]
fn test_is_sub_path_works_with_non_existing_paths() {
    let path = PathBuf::from(r"C:\Some/Non-existing\Path/Whatever.exe");
    let parent = PathBuf::from(r"c:\some\non-existing/path\");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Some/Non-existing\Path/Whatever.exe");
    let parent = PathBuf::from(r"c:\some\non-existing/path/");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Some/Non-existing\Path/Whatever.exe");
    let parent = PathBuf::from(r"c:\some/non-existing/");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\AppData\JamLogic");
    let parent = PathBuf::from(r"C:\AppData\JamLogicDev");
    assert!(!is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\AppData\JamLogicDev");
    let parent = PathBuf::from(r"C:\AppData\JamLogic");
    assert!(!is_sub_path(&path, &parent).unwrap());
}

#[test]
fn test_is_sub_path_works_with_env_var_paths_and_avoids_current_dir() {
    let path = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    let parent = PathBuf::from(r"%windir%");
    assert!(is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from(r"C:\Source\rust setup testing\install");
    let parent = PathBuf::from(r"%windir%\system32");
    assert!(!is_sub_path(&path, &parent).unwrap());

    let path = r"%windir%\system32";
    let parent = std::env::current_dir().unwrap().to_string_lossy().to_string();
    assert!(!is_sub_path(&path, &parent).unwrap());
    assert!(!is_sub_path(&parent, &path).unwrap());
}

#[test]
fn test_is_sub_path_works_with_empty_paths() {
    let path = PathBuf::from(r"C:\Windows\Path.exe");
    let parent = PathBuf::from("");
    assert!(!is_sub_path(&path, &parent).unwrap());

    let path = PathBuf::from("");
    let parent = PathBuf::from(r"c:\some\non-existing/path/");
    assert!(!is_sub_path(&path, &parent).unwrap());
}

pub fn is_os_version_or_greater(version: &str) -> Result<bool> {
    let (mut major, mut minor, mut build, _) = shared::parse_version(version)?;

    if major < 8 {
        return Ok(w::IsWindows7OrGreater()?);
    }

    if major == 8 {
        return Ok(if minor >= 1 { w::IsWindows8Point1OrGreater()? } else { w::IsWindows8OrGreater()? });
    }

    // https://en.wikipedia.org/wiki/List_of_Microsoft_Windows_versions
    if major == 11 {
        if build < 22000 {
            build = 22000;
        }
        major = 10;
        minor = 0;
    }

    if major == 10 && build <= 0 {
        return Ok(w::IsWindows10OrGreater()?);
    }

    let mut mask: u64 = 0;
    mask = w::VerSetConditionMask(mask, co::VER_MASK::MAJORVERSION, co::VER_COND::GREATER_EQUAL);
    mask = w::VerSetConditionMask(mask, co::VER_MASK::MINORVERSION, co::VER_COND::GREATER_EQUAL);
    mask = w::VerSetConditionMask(mask, co::VER_MASK::BUILDNUMBER, co::VER_COND::GREATER_EQUAL);

    let mut osvi: w::OSVERSIONINFOEX = Default::default();
    osvi.dwMajorVersion = major;
    osvi.dwMinorVersion = minor;
    osvi.dwBuildNumber = build;
    return Ok(w::VerifyVersionInfo(&mut osvi, co::VER_MASK::MAJORVERSION | co::VER_MASK::MINORVERSION | co::VER_MASK::BUILDNUMBER, mask)?);
}

#[test]
#[ignore]
pub fn test_os_returns_true_for_everything_on_windows_11_and_below() {
    assert!(is_os_version_or_greater("6").unwrap());
    assert!(is_os_version_or_greater("7").unwrap());
    assert!(is_os_version_or_greater("8").unwrap());
    assert!(is_os_version_or_greater("8.1").unwrap());
    assert!(is_os_version_or_greater("10").unwrap());
    assert!(is_os_version_or_greater("10.0.20000").unwrap());
    assert!(is_os_version_or_greater("11").unwrap());
    assert!(!is_os_version_or_greater("12").unwrap());
}

pub fn is_cpu_architecture_supported(architecture: &str) -> Result<bool> {
    let machine = RuntimeArch::from_current_system();
    if machine.is_none() {
        // we can't detect current os arch so try installing anyway
        return Ok(true);
    }

    let architecture = RuntimeArch::from_str(architecture);
    if architecture.is_none() {
        // no arch specified in this package, so install on any arch
        return Ok(true);
    }

    let machine = machine.unwrap();
    let architecture = architecture.unwrap();
    let is_win_11 = is_os_version_or_greater("11")?;

    if machine == RuntimeArch::X86 {
        // windows x86 only supports x86
        Ok(architecture == RuntimeArch::X86)
    } else if machine == RuntimeArch::X64 {
        // windows x64 only supports x86 and x64
        Ok(architecture == RuntimeArch::X86 || architecture == RuntimeArch::X64)
    } else if machine == RuntimeArch::Arm64 {
        // windows arm64 supports x86, and arm64, and only on Windows 11 does it support x64
        Ok(architecture == RuntimeArch::X86 || (architecture == RuntimeArch::X64 && is_win_11) || architecture == RuntimeArch::Arm64)
    } else {
        // we don't know what this is, so try installing anyway
        Ok(true)
    }
}

#[test]
pub fn test_x64_and_x86_is_supported_but_not_arm64_or_invalid() {
    assert!(!is_cpu_architecture_supported("arm64").unwrap());
    assert!(is_cpu_architecture_supported("invalid").unwrap());
    assert!(is_cpu_architecture_supported("x64").unwrap());
    assert!(is_cpu_architecture_supported("x86").unwrap());
}
