use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::ErrorKind, path::Path};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Language,
}

impl Preferences {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Cannot read desktop preferences"),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).context("Cannot open desktop preferences"),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        labelize_printer_core::paths::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
}

impl Language {
    pub fn locale(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Chinese => Self::English,
            Self::English => Self::Chinese,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Chinese => "简体中文",
            Self::English => "English",
        }
    }

    pub fn page_count(self, count: usize) -> String {
        match self {
            Self::Chinese => format!("{count} 页"),
            Self::English if count == 1 => "1 page".into(),
            Self::English => format!("{count} pages"),
        }
    }

    pub fn text(self, text: Text) -> &'static str {
        let (chinese, english) = match text {
            Text::Studio => ("工作台", "Workspace"),
            Text::StudioDescription => ("打开、编辑和预览标签文件，设计标签并保存 PDF / PNG。", "Open, edit and preview label files, design labels and save PDF / PNG."),
            Text::Setup => ("系统打印机", "System printer"),
            Text::Settings => ("设置", "Settings"),
            Text::Jobs => ("任务记录", "Jobs"),
            Text::SetupDescription => ("添加系统打印机，或先用一份 ZPL 测试输出。", "Add the system printer or try a ZPL test print."),
            Text::SettingsDescription => ("选择文件格式、保存目录和缺省标签尺寸。", "Choose the file format, output folder and default label size."),
            Text::JobsDescription => ("查看任务状态，打开已保存的文件。", "Check job status and open saved files."),
            Text::EnvironmentRefreshed => ("打印环境已刷新。", "Printer detection refreshed."),
            Text::SettingsSaved => ("设置已保存，将用于新任务。", "Settings saved for new print jobs."),
            Text::PrinterConfigured => ("打印机已配置，可以从系统打印窗口选择 Labelize Printer。", "Printer configured. Select Labelize Printer in the system print dialog."),
            Text::TestSubmitted => ("测试任务已提交。文件写入后会显示为「已保存」。", "Test submitted. The job will show as saved once its files are written."),
            Text::JobRetried => ("任务已重新排队。", "Job queued for another attempt."),
            Text::PrinterRemoved => ("系统打印队列已移除。", "System print queue removed."),
            Text::WorkerStopped => ("后台连接线程已退出，请重新打开应用。", "The background connection stopped. Reopen the app to reconnect."),
            Text::InvalidWidth => ("请输入有效的标签宽度。", "Enter a valid label width."),
            Text::InvalidHeight => ("请输入有效的标签高度。", "Enter a valid label height."),
            Text::InvalidDirectory => ("请选择一个完整的输出目录路径。", "Choose an absolute path for the output folder."),
            Text::InvalidMedia => ("标签宽度和高度必须为大于 0 且不超过 4000 的毫米数。", "Label width and height must be greater than 0 and at most 4000 mm."),
            Text::ChooseOutputFolder => ("选择输出目录", "Choose output folder"),
            Text::FolderPickerError => ("无法打开目录选择器。", "Could not open the folder picker."),
            Text::ConfirmRemove => ("移除 Labelize Printer？", "Remove Labelize Printer?"),
            Text::ConfirmRemoveDescription => ("系统打印窗口将移除这台打印机，已保存的文件会保留。", "This removes the printer from system print dialogs. Your saved files are retained."),
            Text::Cancel => ("取消", "Cancel"),
            Text::RemoveQueue => ("移除队列", "Remove queue"),
            Text::MissingPath => ("此文件或目录尚未创建，或已被移动。", "This file or folder has not been created or has been moved."),
            Text::OpenPathError => ("无法打开文件或目录。", "Could not open the file or folder."),
            Text::Detecting => ("检测中…", "Detecting…"),
            Text::Refresh => ("刷新检测", "Refresh detection"),
            Text::Connect => ("连接后台", "Reconnect"),
            Text::PdfFormat => ("PDF · 每个任务一份", "PDF · one file per job"),
            Text::PngFormat => ("PNG · 每页一张", "PNG · one file per page"),
            Text::Browse => ("选择目录…", "Browse…"),
            Text::FilesAndLabels => ("文件与标签", "Files and labels"),
            Text::OutputFormat => ("输出格式", "Output format"),
            Text::OutputFolder => ("保存目录", "Output folder"),
            Text::LabelSize => ("缺省宽度 × 高度（mm）", "Default width × height (mm)"),
            Text::ZplResolution => ("ZPL 分辨率", "ZPL resolution"),
            Text::DriverResolution => ("分辨率随所选驱动调整。", "Resolution follows the selected driver."),
            Text::MediaDescription => ("ZPL 声明的尺寸优先。系统栅格保留原尺寸。", "ZPL label sizes take precedence. System raster pages keep their original size."),
            Text::AutostartOn => ("登录时自动启动已开启", "Start at login enabled"),
            Text::AutostartOff => ("登录时自动启动已关闭", "Start at login disabled"),
            Text::Connected => ("后台已连接", "Connected"),
            Text::Disconnected => ("后台未连接", "Disconnected"),
            Text::Connecting => ("正在连接后台…", "Connecting…"),
            Text::QueueAdded => ("系统打印机已添加", "Printer added"),
            Text::QueueMissing => ("系统打印机尚未添加", "Printer not added"),
            Text::SystemPrinter => ("系统打印机", "System printer"),
            Text::InstallDriver => ("安装官方 ZDesigner ZPL 驱动后，点击「刷新检测」。", "Install the official ZDesigner ZPL driver, then refresh detection."),
            Text::DownloadDriver => ("Zebra 驱动下载", "Download Zebra driver"),
            Text::DriverDownloadError => ("无法打开驱动下载页。", "Could not open the driver download page."),
            Text::ZplDriver => ("ZDesigner ZPL 驱动", "ZDesigner ZPL driver"),
            Text::SelectDriver => ("选择驱动", "Select a driver"),
            Text::ChooseResolution => ("请在下方选择与驱动一致的 ZPL 分辨率。", "Choose the ZPL resolution that matches your driver below."),
            Text::DisableStatusCheck => ("在驱动设置中将 Status check type 设置为 Disabled。", "Set Status check type to Disabled in the driver settings."),
            Text::SystemQueue => ("系统队列", "System queue"),
            Text::BackgroundPrinting => ("关闭窗口后，后台仍可接收打印。", "The background service keeps accepting jobs when this window is closed."),
            Text::RemovePrinter => ("移除打印机…", "Remove printer…"),
            Text::Submitting => ("正在提交…", "Submitting…"),
            Text::TestZpl => ("测试 ZPL 输出", "Test ZPL output"),
            Text::NoJobs => ("暂无任务", "No jobs yet"),
            Text::PrintFirstJob => ("从系统打印窗口选择 Labelize Printer，或提交一份测试标签。", "Select Labelize Printer in a system print dialog or submit a test label."),
            Text::SetupFirstJob => ("先测试文件输出，或到安装向导添加系统打印机。", "Try a test print or add the system printer on the Setup page."),
            Text::Receiving => ("接收中", "Receiving"),
            Text::Queued => ("排队中", "Queued"),
            Text::Processing => ("处理中", "Processing"),
            Text::Saved => ("已保存", "Saved"),
            Text::Failed => ("失败", "Failed"),
            Text::SystemPrint => ("系统打印", "System print"),
            Text::TestSource => ("测试 ZPL", "Test ZPL"),
            Text::DesignSource => ("标签设计", "Label design"),
            Text::OpenFolder => ("打开目录", "Open folder"),
            Text::OpenFirstPage => ("打开第 1 页", "Open first page"),
            Text::OpenFile => ("打开文件", "Open file"),
            Text::Retry => ("重试", "Retry"),
            Text::SaveAndTest => ("保存并测试输出", "Save and test"),
            Text::Configuring => ("正在配置…", "Configuring…"),
            Text::RepairPrinter => ("修复打印机", "Repair printer"),
            Text::AddPrinter => ("添加打印机", "Add printer"),
            Text::OpenOutputFolder => ("打开输出目录", "Open output folder"),
            Text::Saving => ("正在保存…", "Saving…"),
            Text::SaveSettings => ("保存设置", "Save settings"),
            Text::ReconnectDescription => ("后台未连接。可重新连接后继续操作。", "The background service is disconnected. Reconnect to continue."),
            Text::SaveLanguageError => ("无法保存语言设置。", "Could not save the language preference."),
            Text::Quit => ("退出", "Quit"),
        };
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

// The catalog also includes messages used only by the Windows setup page.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Text {
    Studio,
    StudioDescription,
    Setup,
    Settings,
    Jobs,
    SetupDescription,
    SettingsDescription,
    JobsDescription,
    EnvironmentRefreshed,
    SettingsSaved,
    PrinterConfigured,
    TestSubmitted,
    JobRetried,
    PrinterRemoved,
    WorkerStopped,
    InvalidWidth,
    InvalidHeight,
    InvalidDirectory,
    InvalidMedia,
    ChooseOutputFolder,
    FolderPickerError,
    ConfirmRemove,
    ConfirmRemoveDescription,
    Cancel,
    RemoveQueue,
    MissingPath,
    OpenPathError,
    Detecting,
    Refresh,
    Connect,
    PdfFormat,
    PngFormat,
    Browse,
    FilesAndLabels,
    OutputFormat,
    OutputFolder,
    LabelSize,
    ZplResolution,
    DriverResolution,
    MediaDescription,
    AutostartOn,
    AutostartOff,
    Connected,
    Disconnected,
    Connecting,
    QueueAdded,
    QueueMissing,
    SystemPrinter,
    InstallDriver,
    DownloadDriver,
    DriverDownloadError,
    ZplDriver,
    SelectDriver,
    ChooseResolution,
    DisableStatusCheck,
    SystemQueue,
    BackgroundPrinting,
    RemovePrinter,
    Submitting,
    TestZpl,
    NoJobs,
    PrintFirstJob,
    SetupFirstJob,
    Receiving,
    Queued,
    Processing,
    Saved,
    Failed,
    SystemPrint,
    TestSource,
    DesignSource,
    OpenFolder,
    OpenFirstPage,
    OpenFile,
    Retry,
    SaveAndTest,
    Configuring,
    RepairPrinter,
    AddPrinter,
    OpenOutputFolder,
    Saving,
    SaveSettings,
    ReconnectDescription,
    SaveLanguageError,
    Quit,
}
