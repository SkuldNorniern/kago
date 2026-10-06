//! every line the setup window shows, in English, Korean and Japanese. `{name}`,
//! `{version}`, `{path}` and `{item}` are filled in by [`Lang::fill`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ko,
    Ja,
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
    Update,
    Repair,
    StateRepair,
    StateUpdate,
    StateNewer,
    StateReplace,
    StateToEveryone,
    StateToUser,
    Agree,
    LicenseTitle,
    LicenseBody,
    LicenseAgreeHint,
    LicenseAccept,
    LicenseRead,
    ScopeTitle,
    ScopeBody,
    InstallFor,
    ForEveryone,
    ForEveryoneNote,
    JustMe,
    Everyone,
    JustMeNote,
    AdminDeclined,
    AlreadyRunning,
    SpaceNote,
    SpaceShort,
    SpaceRequired,
    SpaceAvailable,
    RunningTitle,
    RunningBody,
    RunningNote,
    UninstallRunning,
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
    NotFound,
    UninstallTitle,
    UninstallAsk,
    InstalledIn,
    Uninstall,
    Removing,
    Removed,
    RemovedPartly,
    RemovedRestart,
    AtRestart,
    Kept,
    NotRemoved,
    UninstallFailed,
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
    Setting,
    Shortcut,
    Register,
    Hook,
    Service,
    Prerequisite,
    Firewall,
    /// Next without the arrow, for the modern look.
    Continue,
    FieldRequired,
    FieldNumber,
    FieldMin,
    FieldMax,
    FieldOption,
}

impl Lang {
    /// `KAGO_LANG` when set, like `ja`, else the system's language when
    /// there are lines for it, else English.
    #[must_use]
    pub fn detect() -> Self {
        std::env::var("KAGO_LANG")
            .ok()
            .or_else(sys_locale::get_locale)
            .map_or(Self::En, |locale| Self::of(&locale))
    }

    /// the language of a locale like `ko-KR` or `ja_JP`.
    #[must_use]
    pub fn of(locale: &str) -> Self {
        let locale = locale.to_ascii_lowercase();
        if locale.starts_with("ko") {
            Self::Ko
        } else if locale.starts_with("ja") {
            Self::Ja
        } else {
            Self::En
        }
    }

    #[must_use]
    pub const fn line(self, line: Line) -> &'static str {
        let [en, ko, ja] = table(line);
        match self {
            Self::En => en,
            Self::Ko => ko,
            Self::Ja => ja,
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

    /// bytes as a short size, like `12.3 MB`. the units read the same in
    /// both languages.
    #[must_use]
    pub fn size(self, bytes: u64) -> String {
        const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
        #[allow(clippy::cast_precision_loss)]
        let mut value = bytes as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < UNITS.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        let _ = self;
        if unit == 0 || value >= 100.0 {
            format!("{value:.0} {}", UNITS[unit])
        } else {
            format!("{value:.1} {}", UNITS[unit])
        }
    }

    /// why an answer does not fit its field, in this language.
    #[must_use]
    pub fn problem(self, problem: crate::manifest::Problem) -> String {
        use crate::manifest::Problem;
        match problem {
            Problem::Empty => self.line(Line::FieldRequired).to_owned(),
            Problem::NotNumber => self.line(Line::FieldNumber).to_owned(),
            Problem::TooSmall(n) => self.fill(Line::FieldMin, &[("item", &n.to_string())]),
            Problem::TooBig(n) => self.fill(Line::FieldMax, &[("item", &n.to_string())]),
            Problem::NotAnOption => self.line(Line::FieldOption).to_owned(),
        }
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
                Self::Ja => "Yu Gothic UI",
            }
        } else if cfg!(target_os = "macos") {
            match self {
                Self::En => "Helvetica",
                Self::Ko => "AppleSDGothicNeo",
                Self::Ja => "Hiragino Sans",
            }
        } else {
            match self {
                Self::En => "Sans",
                Self::Ko => "Noto Sans CJK KR",
                Self::Ja => "Noto Sans CJK JP",
            }
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one entry per line of text, in every language"
)]
const fn table(line: Line) -> [&'static str; 3] {
    match line {
        Line::WindowTitle => ["{name} Setup", "{name} 설치", "{name} セットアップ"],
        Line::WelcomeTitle => [
            "Welcome to {name} Setup",
            "{name} 설치를 시작합니다",
            "{name} セットアップへようこそ",
        ],
        Line::WelcomeBody => [
            "Setup will guide you through the installation of {name} {version}.",
            "{name} {version} 설치를 안내합니다.",
            "{name} {version} のインストールを案内します。",
        ],
        Line::InstallInto => ["Install to", "설치 위치", "インストール先"],
        Line::ClickNext => [
            "Click Next to continue.",
            "계속하려면 다음을 누르세요.",
            "続けるには [次へ] をクリックしてください。",
        ],
        Line::By => ["by {name}", "{name} 제공", "提供: {name}"],
        Line::Next => ["Next >", "다음 >", "次へ >"],
        Line::Back => ["< Back", "< 뒤로", "< 戻る"],
        Line::Return => ["Back", "뒤로", "戻る"],
        Line::Cancel => ["Cancel", "취소", "キャンセル"],
        Line::Install => ["Install", "설치", "インストール"],
        Line::Update => ["Update", "업데이트", "更新"],
        Line::Repair => ["Repair", "복구", "修復"],
        Line::StateRepair => [
            "{name} {version} is already installed. Setup will repair it.",
            "{name} {version}이(가) 이미 설치되어 있습니다. 설치 프로그램이 복구합니다.",
            "{name} {version} はすでにインストールされています。修復します。",
        ],
        Line::StateUpdate => [
            "{name} {from} is installed. Setup will update it to {version}.",
            "{name} {from}이(가) 설치되어 있습니다. {version}(으)로 업데이트합니다.",
            "{name} {from} がインストールされています。{version} に更新します。",
        ],
        Line::StateNewer => [
            "A newer {name} ({from}) is installed. This setup cannot replace it.",
            "더 새로운 {name}({from})이(가) 설치되어 있어 이 설치 프로그램으로 바꿀 수 없습니다.",
            "新しい {name} ({from}) がインストールされているため、このセットアップでは置き換えられません。",
        ],
        Line::StateReplace => [
            "A newer {name} ({from}) is installed. Setup will replace it with {version}.",
            "더 새로운 {name}({from})이(가) 설치되어 있습니다. {version}(으)로 바꿉니다.",
            "新しい {name} ({from}) がインストールされています。{version} に置き換えます。",
        ],
        Line::StateToEveryone => [
            "{name} is installed just for you. Setup will move it to Program Files, for everyone.",
            "{name}이(가) 현재 사용자에게만 설치되어 있습니다. 모든 사용자를 위해 Program Files로 옮깁니다.",
            "{name} は現在のユーザーのみにインストールされています。すべてのユーザー向けに Program Files へ移動します。",
        ],
        Line::StateToUser => [
            "{name} is installed for everyone. Setup will install it just for you and remove that copy.",
            "{name}이(가) 모든 사용자용으로 설치되어 있습니다. 현재 사용자용으로 설치하고 그 사본은 제거합니다.",
            "{name} はすべてのユーザー向けにインストールされています。現在のユーザー向けにインストールし、そのコピーを削除します。",
        ],
        Line::Agree => ["I Agree", "동의함", "同意する"],
        Line::LicenseTitle => ["License Agreement", "사용권 계약", "使用許諾契約"],
        Line::LicenseBody => [
            "Please review the license terms before installing {name}.",
            "{name}을(를) 설치하기 전에 사용권 계약을 확인하세요.",
            "{name} をインストールする前に、使用許諾契約を確認してください。",
        ],
        Line::LicenseAgreeHint => [
            "If you accept the terms of the agreement, click I Agree to continue.",
            "계약 조건에 동의하면 동의함을 눌러 계속하세요.",
            "契約の条件に同意する場合は、[同意する] をクリックして続けてください。",
        ],
        Line::LicenseAccept => [
            "I accept the license agreement",
            "사용권 계약에 동의합니다",
            "使用許諾契約に同意します",
        ],
        Line::LicenseRead => ["Read license", "사용권 계약 보기", "使用許諾契約を読む"],
        Line::ScopeTitle => ["Choose Users", "사용자 선택", "ユーザーの選択"],
        Line::ScopeBody => [
            "Choose for which users you want to install {name}.",
            "{name}을(를) 설치할 사용자를 선택하세요.",
            "{name} をインストールするユーザーを選択してください。",
        ],
        Line::InstallFor => ["Install for", "설치 대상", "インストール対象"],
        Line::ForEveryone => [
            "Anyone who uses this computer",
            "이 컴퓨터를 사용하는 모든 사용자",
            "このコンピューターを使用するすべてのユーザー",
        ],
        Line::ForEveryoneNote => [
            "Into Program Files. Windows asks for administrator rights.",
            "Program Files에 설치합니다. 관리자 권한을 요청합니다.",
            "Program Files にインストールします。管理者権限を求められます。",
        ],
        Line::JustMe => ["Only for me", "현재 사용자만", "自分のみ"],
        Line::Everyone => ["Everyone", "모든 사용자", "すべてのユーザー"],
        Line::JustMeNote => [
            "Into your own folder. No administrator rights needed.",
            "내 사용자 폴더에 설치합니다. 관리자 권한이 필요 없습니다.",
            "自分のフォルダーにインストールします。管理者権限は不要です。",
        ],
        Line::AdminDeclined => [
            "Administrator rights were not given. Install only for you, or try again.",
            "관리자 권한을 받지 못했습니다. 현재 사용자용으로 설치하거나 다시 시도하세요.",
            "管理者権限が得られませんでした。自分のみにインストールするか、もう一度お試しください。",
        ],
        Line::AlreadyRunning => [
            "{name} is being installed or removed already. Wait for that to finish.",
            "{name}을(를) 이미 설치하거나 제거하는 중입니다. 끝날 때까지 기다리세요.",
            "{name} はすでにインストールまたは削除の途中です。終わるまでお待ちください。",
        ],
        Line::SpaceNote => [
            "{needed} needed  \u{b7}  {free} free",
            "필요 {needed}  \u{b7}  남은 공간 {free}",
            "必要 {needed}  \u{b7}  空き {free}",
        ],
        Line::SpaceShort => [
            "Not enough space on this drive: {needed} needed, {free} free.",
            "이 드라이브에 공간이 부족합니다. 필요 {needed}, 남은 공간 {free}.",
            "このドライブの空き容量が足りません。必要 {needed}、空き {free}。",
        ],
        Line::SpaceRequired => [
            "Space required: {item}",
            "필요한 공간: {item}",
            "必要な容量: {item}",
        ],
        Line::SpaceAvailable => [
            "Space available: {item}",
            "사용 가능한 공간: {item}",
            "空き容量: {item}",
        ],
        Line::RunningTitle => [
            "Close Applications",
            "프로그램 닫기",
            "アプリケーションを閉じる",
        ],
        Line::RunningBody => [
            "These programs use files Setup has to replace.",
            "설치 프로그램이 바꿔야 하는 파일을 다음 프로그램이 사용하고 있습니다.",
            "セットアップが置き換えるファイルを、次のプログラムが使用しています。",
        ],
        Line::RunningNote => [
            "Setup will close them. Save your work in them first.",
            "설치 프로그램이 이 프로그램들을 닫습니다. 먼저 작업을 저장하세요.",
            "セットアップがこれらを閉じます。先に作業を保存してください。",
        ],
        Line::UninstallRunning => [
            "These programs will be closed: {item}",
            "다음 프로그램을 닫습니다: {item}",
            "次のプログラムを閉じます: {item}",
        ],
        Line::ComponentsTitle => [
            "Choose Components",
            "구성 요소 선택",
            "コンポーネントの選択",
        ],
        Line::ComponentsBody => [
            "Choose which features of {name} you want to install.",
            "설치할 {name} 기능을 선택하세요.",
            "インストールする {name} の機能を選択してください。",
        ],
        Line::Components => ["Components", "구성 요소", "コンポーネント"],
        Line::DirectoryTitle => [
            "Choose Install Location",
            "설치 위치 선택",
            "インストール先の選択",
        ],
        Line::DirectoryBody => [
            "Choose the folder in which to install {name}.",
            "{name}을(를) 설치할 폴더를 선택하세요.",
            "{name} をインストールするフォルダーを選択してください。",
        ],
        Line::DirectoryHint => [
            "Setup will install {name} in the following folder. To install in a different folder, click Browse and select another folder.",
            "{name}을(를) 다음 폴더에 설치합니다. 다른 폴더에 설치하려면 찾아보기를 눌러 다른 폴더를 선택하세요.",
            "{name} を次のフォルダーにインストールします。別のフォルダーにインストールするには、[参照] をクリックしてフォルダーを選択してください。",
        ],
        Line::Browse => ["Browse...", "찾아보기...", "参照..."],
        Line::Change => ["Change", "변경", "変更"],
        Line::UpdateHere => [
            "Installed here now; the update stays in this folder.",
            "지금 설치된 위치입니다. 업데이트도 이 폴더에 설치됩니다.",
            "現在ここにインストールされています。更新もこのフォルダーに入ります。",
        ],
        Line::FolderNotAbsolute => [
            "Enter a full folder path.",
            "전체 폴더 경로를 입력하세요.",
            "フォルダーのフルパスを入力してください。",
        ],
        Line::FolderRoot => [
            "A drive root cannot hold an install. Pick a folder.",
            "드라이브 루트에는 설치할 수 없습니다. 폴더를 선택하세요.",
            "ドライブのルートにはインストールできません。フォルダーを選択してください。",
        ],
        Line::FolderUnclear => [
            "The folder path must not contain . or ..",
            "폴더 경로에 . 또는 .. 이 들어갈 수 없습니다.",
            "フォルダーのパスに . や .. は使えません。",
        ],
        Line::FolderNotEmpty => [
            "This folder already has files. Pick an empty or new folder.",
            "이 폴더에 이미 파일이 있습니다. 비어 있거나 새 폴더를 선택하세요.",
            "このフォルダーにはすでにファイルがあります。空のフォルダーか新しいフォルダーを選択してください。",
        ],
        Line::NotFound => [
            "{item} not found",
            "{item}을(를) 찾을 수 없음",
            "{item} が見つかりません",
        ],
        Line::UninstallTitle => [
            "Uninstall {name}",
            "{name} 제거",
            "{name} のアンインストール",
        ],
        Line::UninstallAsk => [
            "{name} {version} and everything its setup installed will be removed from this computer.",
            "{name} {version}과(와) 설치 프로그램이 설치한 항목을 이 컴퓨터에서 제거합니다.",
            "{name} {version} と、そのセットアップがインストールしたものをすべてこのコンピューターから削除します。",
        ],
        Line::InstalledIn => ["Installed in", "설치된 위치", "インストール先"],
        Line::Uninstall => ["Uninstall", "제거", "アンインストール"],
        Line::Removing => [
            "Removing {name}...",
            "{name} 제거 중...",
            "{name} を削除しています...",
        ],
        Line::Removed => [
            "{name} was removed.",
            "{name}이(가) 제거되었습니다.",
            "{name} を削除しました。",
        ],
        Line::RemovedRestart => [
            "{name} was removed. Restart Windows to finish.",
            "{name}이(가) 제거되었습니다. 마무리하려면 Windows를 다시 시작하세요.",
            "{name} を削除しました。完了するには Windows を再起動してください。",
        ],
        Line::AtRestart => [
            "Still in use, removed when Windows restarts:",
            "사용 중이라 Windows를 다시 시작할 때 제거됩니다:",
            "使用中のため、Windows の再起動時に削除されます:",
        ],
        Line::RemovedPartly => [
            "{name} was removed, but not completely.",
            "{name}이(가) 제거되었지만 일부가 남았습니다.",
            "{name} を削除しましたが、一部が残っています。",
        ],
        Line::Kept => [
            "Kept, because you changed them or they hold your data:",
            "변경했거나 사용자 데이터라서 남겨 둔 항목:",
            "変更されたか、ユーザーのデータのため残した項目:",
        ],
        Line::NotRemoved => [
            "These could not be removed. Close anything using them and run the uninstaller again:",
            "제거하지 못한 항목입니다. 사용 중인 프로그램을 닫고 제거 프로그램을 다시 실행하세요:",
            "次の項目は削除できませんでした。使用しているものを閉じて、アンインストーラーをもう一度実行してください:",
        ],
        Line::UninstallFailed => [
            "Uninstall failed",
            "제거 실패",
            "アンインストールに失敗しました",
        ],
        Line::InstallingTitle => ["Installing", "설치 중", "インストール中"],
        Line::InstallingBody => [
            "Please wait while {name} is being installed.",
            "{name}을(를) 설치하는 동안 기다려 주세요.",
            "{name} をインストールしています。しばらくお待ちください。",
        ],
        Line::DoneTitle => [
            "{name} is installed",
            "{name} 설치 완료",
            "{name} のインストールが完了しました",
        ],
        Line::DoneBody => [
            "{name} has been installed on your computer.",
            "{name}이(가) 컴퓨터에 설치되었습니다.",
            "{name} がコンピューターにインストールされました。",
        ],
        Line::RunAfter => ["Run {name}", "{name} 실행", "{name} を実行"],
        Line::Open => ["Open {name}", "{name} 열기", "{name} を開く"],
        Line::Finish => ["Finish", "마침", "完了"],
        Line::Close => ["Close", "닫기", "閉じる"],
        Line::FailedTitle => ["Setup failed", "설치 실패", "セットアップに失敗しました"],
        Line::FailedBody => [
            "Nothing was changed. The error was:",
            "변경된 내용은 없습니다. 오류:",
            "何も変更されていません。エラー:",
        ],
        Line::Unpack => ["Extract: {item}", "압축 해제: {item}", "展開: {item}"],
        Line::Folder => [
            "Create folder: {item}",
            "폴더 만들기: {item}",
            "フォルダーの作成: {item}",
        ],
        Line::File => ["Install: {item}", "설치: {item}", "インストール: {item}"],
        Line::Path => [
            "Add to PATH: {item}",
            "PATH에 추가: {item}",
            "PATH に追加: {item}",
        ],
        Line::Setting => ["Set: {item}", "설정: {item}", "設定: {item}"],
        Line::Shortcut => [
            "Create shortcut: {item}",
            "바로 가기 만들기: {item}",
            "ショートカットの作成: {item}",
        ],
        Line::Hook => ["Run: {item}", "실행: {item}", "実行: {item}"],
        Line::Register => ["Register {item}", "{item} 등록", "{item} を登録"],
        Line::Service => [
            "Start at sign-in: {item}",
            "로그인 시 시작: {item}",
            "サインイン時に開始: {item}",
        ],
        Line::Prerequisite => [
            "Install first: {item}",
            "먼저 설치: {item}",
            "先にインストール: {item}",
        ],
        Line::Firewall => [
            "Firewall rule: {item}",
            "방화벽 규칙: {item}",
            "ファイアウォール規則: {item}",
        ],
        Line::Continue => ["Next", "다음", "次へ"],
        Line::FieldRequired => [
            "Fill this in to go on.",
            "계속하려면 입력하세요.",
            "続けるには入力してください。",
        ],
        Line::FieldNumber => [
            "Enter a whole number.",
            "정수를 입력하세요.",
            "整数を入力してください。",
        ],
        Line::FieldMin => [
            "Enter {item} or more.",
            "{item} 이상을 입력하세요.",
            "{item} 以上を入力してください。",
        ],
        Line::FieldMax => [
            "Enter {item} or less.",
            "{item} 이하를 입력하세요.",
            "{item} 以下を入力してください。",
        ],
        Line::FieldOption => [
            "Pick one of the options.",
            "선택지 중 하나를 고르세요.",
            "選択肢から選んでください。",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_short() {
        assert_eq!(Lang::En.size(512), "512 B");
        assert_eq!(Lang::En.size(1536), "1.5 KB");
        assert_eq!(Lang::Ko.size(3_751_936), "3.6 MB");
        assert_eq!(Lang::En.size(400 * 1024 * 1024), "400 MB");
    }

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
        assert_eq!(
            Lang::Ja.fill(Line::Removed, &[("name", "Ozone")]),
            "Ozone を削除しました。"
        );
    }

    #[test]
    fn locales_pick_their_language() {
        assert_eq!(Lang::of("ko-KR"), Lang::Ko);
        assert_eq!(Lang::of("ja_JP"), Lang::Ja);
        assert_eq!(Lang::of("de-DE"), Lang::En);
    }
}
