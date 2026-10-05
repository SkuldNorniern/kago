//! `.lnk` files through the shell's `IShellLinkW`.

use std::path::Path;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface};

pub fn create(link: &Path, target: &Path, arguments: &[String]) -> Result<(), String> {
    let err = |e: windows::core::Error| format!("cannot create shortcut {}: {e}", link.display());
    // SAFETY: COM is initialized for this thread and released below; every
    // string passed lives across its call.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell.SetPath(&HSTRING::from(target))?;
            if let Some(dir) = target.parent() {
                shell.SetWorkingDirectory(&HSTRING::from(dir))?;
            }
            if !arguments.is_empty() {
                shell.SetArguments(&HSTRING::from(join_arguments(arguments)))?;
            }
            shell.SetIconLocation(&HSTRING::from(target), 0)?;
            let file: IPersistFile = shell.cast()?;
            file.Save(&HSTRING::from(link), true)
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result.map_err(err)
    }
}

/// one argument quoted for a windows command line.
pub fn quote(a: &str) -> String {
    join_arguments(&[a.to_owned()])
}

/// windows command-line quoting for each argument.
pub fn join_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|a| {
            if !a.is_empty() && !a.contains([' ', '\t', '"']) {
                return a.clone();
            }
            let mut out = String::from('"');
            let mut backslashes = 0;
            for c in a.chars() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        out.push_str(&"\\".repeat(backslashes * 2 + 1));
                        out.push('"');
                        backslashes = 0;
                    }
                    _ => {
                        out.push_str(&"\\".repeat(backslashes));
                        out.push(c);
                        backslashes = 0;
                    }
                }
            }
            out.push_str(&"\\".repeat(backslashes * 2));
            out.push('"');
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_like_windows() {
        let a = |v: &[&str]| join_arguments(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(a(&["--x", "y"]), "--x y");
        assert_eq!(a(&["a b"]), "\"a b\"");
        assert_eq!(a(&[""]), "\"\"");
        assert_eq!(a(&["say \"hi\""]), "\"say \\\"hi\\\"\"");
        assert_eq!(a(&["C:\\dir with space\\"]), "\"C:\\dir with space\\\\\"");
    }
}
