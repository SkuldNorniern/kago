//! every line the setup window shows, in English and Korean. `{name}`,
//! `{version}`, `{path}` and `{item}` are filled in by [`Lang::fill`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    WindowTitle,
    WelcomeTitle,
    WelcomeBody,
    InstallInto,
    ClickNext,
    By,
    Next,
    Back,
    /// Back without the arrow, for the modern look.
    Return,
    Cancel,
    Install,
    Agree,
    LicenseTitle,
    LicenseBody,
    LicenseAgreeHint,
    LicenseAccept,
    LicenseRead,
    ComponentsTitle,
    ComponentsBody,
    Components,
    DirectoryTitle,
    DirectoryBody,
    DirectoryHint,
    Browse,
    Change,
    UpdateHere,
    FolderNotAbsolute,
    FolderRoot,
    FolderUnclear,
    FolderNotEmpty,
    InstallingTitle,
    InstallingBody,
    DoneTitle,
    DoneBody,
    RunAfter,
    Open,
    Finish,
    Close,
    FailedTitle,
    FailedBody,
    Unpack,
    Folder,
    File,
    Path,
    SteamVr,
    Shortcut,
    Register,
    Service,
}

impl Lang {
    /// Korean when the system says so, English otherwise.
    #[must_use]
    pub fn detect() -> Self {
        match sys_locale::get_locale() {
            Some(locale) if locale.to_ascii_lowercase().starts_with("ko") => Self::Ko,
            _ => Self::En,
        }
    }

    #[must_use]
    pub const fn line(self, line: Line) -> &'static str {
        let [en, ko] = table(line);
        match self {
            Self::En => en,
            Self::Ko => ko,
        }
    }

    /// `line` with each `{key}` replaced by its value.
    #[must_use]
    pub fn fill(self, line: Line, values: &[(&str, &str)]) -> String {
        let mut text = self.line(line).to_owned();
        for (key, value) in values {
            text = text.replace(&format!("{{{key}}}"), value);
        }
        text
    }

    /// why a folder cannot be used, in this language.
    #[must_use]
    pub const fn folder_error(self, error: crate::install::LocationError) -> &'static str {
        use crate::install::LocationError;
        self.line(match error {
            LocationError::NotAbsolute => Line::FolderNotAbsolute,
            LocationError::Root => Line::FolderRoot,
            LocationError::Unclear => Line::FolderUnclear,
            LocationError::NotEmpty => Line::FolderNotEmpty,
        })
    }

    /// family that has this language's glyphs, for canvas text.
    #[must_use]
    pub const fn font_family(self) -> &'static str {
        if cfg!(windows) {
            match self {
                Self::En => "Segoe UI",
                Self::Ko => "Malgun Gothic",
            }
        } else if cfg!(target_os = "macos") {
            match self {
                Self::En => "Helvetica",
                Self::Ko => "AppleSDGothicNeo",
            }
        } else {
            match self {
                Self::En => "Sans",
                Self::Ko => "Noto Sans CJK KR",
            }
        }
    }
}

const fn table(line: Line) -> [&'static str; 2] {
    match line {
        Line::WindowTitle => ["{name} Setup", "{name} 설치"],
        Line::WelcomeTitle => ["Welcome to {name} Setup", "{name} 설치를 시작합니다"],
        Line::WelcomeBody => [
            "Setup will guide you through the installation of {name} {version}.",
            "{name} {version} 설치를 안내합니다.",
        ],
        Line::InstallInto => ["Install to", "설치 위치"],
        Line::ClickNext => ["Click Next to continue.", "계속하려면 다음을 누르세요."],
        Line::By => ["by {name}", "{name} 제공"],
        Line::Next => ["Next >", "다음 >"],
        Line::Back => ["< Back", "< 뒤로"],
        Line::Return => ["Back", "뒤로"],
        Line::Cancel => ["Cancel", "취소"],
        Line::Install => ["Install", "설치"],
        Line::Agree => ["I Agree", "동의함"],
        Line::LicenseTitle => ["License Agreement", "사용권 계약"],
        Line::LicenseBody => [
            "Please review the license terms before installing {name}.",
            "{name}을(를) 설치하기 전에 사용권 계약을 확인하세요.",
        ],
        Line::LicenseAgreeHint => [
            "If you accept the terms of the agreement, click I Agree to continue.",
            "계약 조건에 동의하면 동의함을 눌러 계속하세요.",
        ],
        Line::LicenseAccept => ["I accept the license agreement", "사용권 계약에 동의합니다"],
        Line::LicenseRead => ["Read license", "사용권 계약 보기"],
        Line::ComponentsTitle => ["Choose Components", "구성 요소 선택"],
        Line::ComponentsBody => [
            "Choose which features of {name} you want to install.",
            "설치할 {name} 기능을 선택하세요.",
        ],
        Line::Components => ["Components", "구성 요소"],
        Line::DirectoryTitle => ["Choose Install Location", "설치 위치 선택"],
        Line::DirectoryBody => [
            "Choose the folder in which to install {name}.",
            "{name}을(를) 설치할 폴더를 선택하세요.",
        ],
        Line::DirectoryHint => [
            "Setup will install {name} in the following folder. To install in a different folder, click Browse and select another folder.",
            "{name}을(를) 다음 폴더에 설치합니다. 다른 폴더에 설치하려면 찾아보기를 눌러 다른 폴더를 선택하세요.",
        ],
        Line::Browse => ["Browse...", "찾아보기..."],
        Line::Change => ["Change", "변경"],
        Line::UpdateHere => [
            "Installed here now; the update stays in this folder.",
            "지금 설치된 위치입니다. 업데이트도 이 폴더에 설치됩니다.",
        ],
        Line::FolderNotAbsolute => ["Enter a full folder path.", "전체 폴더 경로를 입력하세요."],
        Line::FolderRoot => [
            "A drive root cannot hold an install. Pick a folder.",
            "드라이브 루트에는 설치할 수 없습니다. 폴더를 선택하세요.",
        ],
        Line::FolderUnclear => [
            "The folder path must not contain . or ..",
            "폴더 경로에 . 또는 .. 이 들어갈 수 없습니다.",
        ],
        Line::FolderNotEmpty => [
            "This folder already has files. Pick an empty or new folder.",
            "이 폴더에 이미 파일이 있습니다. 비어 있거나 새 폴더를 선택하세요.",
        ],
        Line::InstallingTitle => ["Installing", "설치 중"],
        Line::InstallingBody => [
            "Please wait while {name} is being installed.",
            "{name}을(를) 설치하는 동안 기다려 주세요.",
        ],
        Line::DoneTitle => ["{name} is installed", "{name} 설치 완료"],
        Line::DoneBody => [
            "{name} has been installed on your computer.",
            "{name}이(가) 컴퓨터에 설치되었습니다.",
        ],
        Line::RunAfter => ["Run {name}", "{name} 실행"],
        Line::Open => ["Open {name}", "{name} 열기"],
        Line::Finish => ["Finish", "마침"],
        Line::Close => ["Close", "닫기"],
        Line::FailedTitle => ["Setup failed", "설치 실패"],
        Line::FailedBody => [
            "Nothing was changed. The error was:",
            "변경된 내용은 없습니다. 오류:",
        ],
        Line::Unpack => ["Extract: {item}", "압축 해제: {item}"],
        Line::Folder => ["Create folder: {item}", "폴더 만들기: {item}"],
        Line::File => ["Install: {item}", "설치: {item}"],
        Line::Path => ["Add to PATH: {item}", "PATH에 추가: {item}"],
        Line::SteamVr => [
            "Register SteamVR driver: {item}",
            "SteamVR 드라이버 등록: {item}",
        ],
        Line::Shortcut => ["Create shortcut: {item}", "바로 가기 만들기: {item}"],
        Line::Register => ["Register {item}", "{item} 등록"],
        Line::Service => ["Install service: {item}", "서비스 설치: {item}"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_every_key() {
        assert_eq!(
            Lang::En.fill(
                Line::WelcomeBody,
                &[("name", "Ozone"), ("version", "0.1.0")]
            ),
            "Setup will guide you through the installation of Ozone 0.1.0."
        );
        assert_eq!(
            Lang::Ko.fill(Line::WindowTitle, &[("name", "Ozone")]),
            "Ozone 설치"
        );
    }
}
