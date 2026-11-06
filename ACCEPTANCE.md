# MVP 验收记录

更新于 2026-11-07。实现和安装包可供内部试用。本记录区分构建、协议自动化、
真实系统打印和目标操作系统实机验收。没有将交叉编译或 CI 配置算作实机通过。

## 当前结果

| 项目 | 结果 | 实际环境 / 边界 |
|---|---|---|
| macOS Apple Silicon 程序和 PKG | 已构建 | 当前 macOS ARM 主机，应用 ad-hoc 签名，PKG 未签名/公证 |
| macOS 原生系统打印 | 已通过 | “预览”的系统打印窗口可选 Labelize Printer；关闭 GPUI 后打印两份，后台保存一份两页 PDF |
| macOS Intel 程序和 PKG | 已构建；后台测试通过 | x86_64 交叉编译，后台和原生 PAPPL 经 Rosetta 执行；尚未在 Intel 实机测试 GPUI |
| Windows x64 程序和 NSIS EXE | UTM 试用通过 | Windows 11 Pro ARM64（Build 26200），x64 仿真。GPUI、安装/升级/卸载、登录启动及直接 RAW 输出已验证；ZDesigner 系统打印待验收 |
| Debian 12 x64 程序、PAPPL 和 DEB | 已构建；后台及系统队列测试通过 | Debian 12 amd64 容器，实际安装 DEB → CUPS 的 PDF 栅格过滤 → PAPPL → 两页 PDF，移除队列和 DEB 成功 |
| Ubuntu 24.04 | 待实机验收 | 提供同一 amd64 DEB；尚未验证安装依赖、登录服务与桌面打印 |
| GPUI 输入、文件选择、缩放 | macOS 部分通过 | 中文路径粘贴、原生目录选择、设置保存、窗口缩放、任务列表和打开输出；中文输入法组合输入仍需验收 |
| Linux Wayland | 容器绘制通过；待桌面验收 | 嵌套 Weston + Mesa 软件 Vulkan 显示完整中文界面及任务结果；真实桌面、目录 portal 和输入法待验收 |
| Linux X11 | 容器绘制通过；待桌面验收 | Xvfb + Openbox + Mesa 软件 Vulkan：首次打开即显示中文界面和任务；包含 GPUI RGB visual 补丁及应用启动时的 configure/restore 兼容处理 |

上述本地试用包使用开发配置构建，并去除 Windows/Linux 分发二进制的调试符号。
发布配置的构建、测试及试用包可查看 [GitHub Actions](https://github.com/GOODBOY008/labelize-printer/actions/workflows/build.yml)。

## 新版桌面界面复测

- Windows 11 UTM 的 800 × 600 桌面可见应用、标题栏和任务栏图标。用键盘切换页面、
  选择 PNG 并提交测试，任务显示为已保存，实际输出 816 × 1216 像素。
  Page Down 可滚动设置区，底部保存按钮保持可见。
- 用新版 NSIS 覆盖升级后，已核对安装程序写入的图标配置及应用文件哈希。
  升级前的 4 条任务、8 个输出文件和输出设置均已保留，后台恢复监听。
- macOS Apple Silicon 上，使用原生目录选择器选择含中文和空格的目录，保存并生成 PNG。
  缩小窗口时可继续操作底部按钮。无效的标签宽度会显示错误，后台断开后可重新连接，
  表单保留未保存的输入。
- Debian 12 安装新版 DEB 后，Xvfb/Openbox 和嵌套 Weston 的 640 × 480
  桌面均能完整显示导航和底部操作。新版 DEB 已再次通过 CUPS 到两页 PDF 的系统打印检查。

## Windows UTM 验证

- NSIS 交互安装和覆盖升级成功。修复了交叉编译包缺少 Common Controls v6 清单、
  调试版依赖构建机着色器路径的问题。修复版在虚拟机的 800 × 600 桌面显示中文向导，
  标题栏和导航可见，后台状态为运行中。
- 使用安装包内的脚本注册当前用户登录任务。退出 Windows 后重新登录，未打开 GPUI 时
  后台已监听本机 RAW 端口，命名管道可查询状态，升级前两条任务仍在。
- 关闭 GPUI 后，将同一份两标签 ZPL 分包发到 RAW 端口。首标签 `^PQ2`，两个任务分别
  保存为一份三页 PDF 和三张 PNG。前两页为 812 × 1218 像素，第三页为 600 × 400 像素，
  对应 101.5 × 152.25 mm 和 75 × 50 mm。PNG 的 pHYs 均为 8000 像素/米。
- NSIS 卸载后，应用文件、后台进程和登录任务已移除。重装并打开 GPUI 后，后台自行启动，
  卸载前的四条任务和输出文件仍在。
- 此虚拟机尚无 ZDesigner。临时通用队列的 RAW 提交在 `OpenPrinter` 返回无效打印机名称，
  已清理测试队列及端口。系统打印窗口到 ZDesigner 的路径仍需在接受
  [Zebra 许可协议](https://www.zebra.com/content/experience-fragments/zebra1/us/en/support/eula/end-user-license-agreement-unrestricted-software/master.html)
  并安装驱动后验收。

## macOS 真实系统打印证据

1. 本地 IPP 队列注册为 `LabelizePrinter`，显示名为 `Labelize Printer`。
2. GPUI 测试页经 ZPL → Labelize → PDF 保存。
3. 用“预览”打开测试 PDF，在原生打印窗口选择 Labelize Printer，份数 2、4 × 6 inch。
4. 关闭 GPUI 窗口，确认 GUI 进程退出，再提交打印。
5. 后台任务 `75d36d97-8730-42d5-b759-70663fd9777f` 成为 Saved，输出文件实际存在。
6. `pdfinfo` 检查为 2 页、每页 288 × 432 pt（101.6 × 152.4 mm）。
   `pdfimages -list` 检查为两幅 812 × 1218 的灰度图，203 dpi。
7. 重新打开 GPUI 后，“最近任务”显示该系统打印任务及其两页输出。

此结果证明当前 macOS 主机上的系统打印 → PAPPL → 栅格输出以及关闭 GUI 后打印，
不等价于 macOS 13 兼容性、所有打印应用或所有纸张设置已经验收。

## 自动化检查

- `cargo test -p labelize-printer-core --locked`：6 个集成测试通过。
  覆盖 ZPL 多标签 / `^PQ` / 逐页尺寸、PDF 页数和 MediaBox、栅格像素与 PNG pHYs、
  设置快照、输出提交恢复、同 ID 重试、接收中断、页数/画布限制和路径边界。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：当前 macOS 构建通过。
  GPUI 的旧间接依赖仍有弃用与 future-incompatibility 提示，未修改其运行行为。
- `python3 scripts/smoke.py`：ARM macOS、Intel/Rosetta 和 Debian 12 amd64 后台通过。
  覆盖真实 RAW TCP 分包及 EOF、`^XZ` 不提前提交、截断任务失败、worker 被杀后重试、
  后台重启、IPP manifest 写失败回报、PWG 1/8-bit、Apple Raster、页序/极性/尺寸、
  IPP copies 和已展开份数、截断栅格、首次启动端口冲突。
- Debian 测试附加 `--disk-full-dir /disk-full-test`：64 KiB 专用 tmpfs 实际触发 ENOSPC，
  未提交输出且任务失败；释放空间后同 ID 重试保存成功。
- PAPPL C 适配器通过 Clang / GCC 的 `-Wall -Wextra -Werror` 构建。
- 安装后的 DEB 在 Debian 12 容器中完成 CUPS 队列创建、两份 PDF 原生栅格过滤、
  保存两页 PDF、队列移除及卸载。可用 `scripts/linux-system-smoke.sh` 在专用容器中复现。
- X11 和 Wayland 启动后的截图已人工检查，包含完整中文界面和已保存任务。
  这不覆盖实机 GPU、中文输入法组合输入、桌面 portal 或用户登录服务。
- Labelize 上游回归：50 个原有 ZPL 解析测试、5 个新解析选项测试、3 个架构测试、
  170 个 golden 测试、3 个 diff-report 测试通过，共 231 项。
  carrier / unit / reverse 三份报告已检查，黄金图片和报告未改变。

后台测试不创建系统队列或登录服务，测试数据使用独立临时目录。
程序内使用的 Labelize 源码提交见 [依赖来源](vendor/labelize/PROVENANCE.md)。

## 完整验收仍需执行

- Windows：在原生 Windows 11 x64 上复测安装及运行。安装官方 ZDesigner v10 后，
  验证 ZD421 ZPL 203/300 dpi、系统打印窗口可选、驱动正常关闭 RAW 连接、中文图形/条码/压缩图形、
  多标签多份数，以及关闭 GPUI 后从系统打印窗口提交任务。
- macOS 13 Intel / Apple Silicon：PKG 安装、首次授权、LaunchAgent 下次登录、升级/卸载，
  真正的中文输入法组合输入和不同显示器缩放；当前主机并非 macOS 13。
- Ubuntu 24.04 / Debian 12 桌面：DEB 安装/升级/卸载、X11/Wayland、Vulkan 驱动、
  目录 portal、中文输入、用户级 systemd 登录启动，以及原生应用打印窗口。
- 三端共同：文件夹权限变更、真实打印取消、磁盘耗尽导致数据库也无法写入后的恢复、
  任务接收时系统掉电，以及完整安装/修复/卸载后没有遗留队列/登录项。

已实现输出失败和重试机制，但没有把上面这些尚未执行的故障场景标成通过。
数据库所在磁盘完全耗尽时，失败状态本身也可能无法落盘；释放空间并重启后按恢复规则处理。
