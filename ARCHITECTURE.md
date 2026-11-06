# 流程与职责

## 接入与处理边界

Windows 应用交给官方 ZDesigner 生成 ZPL，系统 RAW 端口将字节发送到本机后台。
后台只在正常 EOF 后提交任务；不按网络包、换行或 `^XZ` 拆任务。
超时、RST、空连接和超限输入记录为失败，保留已有输入；不提交残缺内容。

macOS/Linux 的系统打印队列向 PAPPL 提交 PWG/Apple Raster。
PAPPL 负责 IPP 生命周期与栅格协议解码。C 适配器负责检查尺寸、行数和格式，
将 1/8 位 K/W/SW 像素统一为灰度 PGM；不缩放或旋转。内置 PAPPL 有一处固定补丁：
1 位输入也保留客户页面头，而不是替换为默认打印机画布。

系统栅格的 `job-pages-per-set` 表示客户端已经展开份数，适配器验证完整页数后保持原顺序。
否则适配器展开 IPP `copies` 一次，并支持整份排序或逐页排序。
ZPL 的 `^PQ` 仅由 Labelize 展开。统一输出模块没有“份数”参数。

## 持久化与状态

```text
Receiving → Queued → Processing → Saved
     │         │          │
     └─────────┴──────────┴──────→ Failed
                                     │ 完整输入可重试，同一任务 ID
                                     └──────────→ Queued
```

RAW 建立连接时、IPP 开始栅格任务时，创建 `JobRecord` 并快照设置。
IPP 先调用 `begin-raster` 获得 UUID，随后提交带同一 UUID 的 manifest，
因此不会在长任务接收期间使用后来修改的输出配置。

SQLite 使用 WAL 和 FULL 同步；原始输入写入应用私有目录，完成 fsync 后才排队。
任务状态、源标识、设置、页数、输出文件、失败原因和重试次数在数据库中保存。
接入端只得到“已接收”时，不能据此显示“已保存”。

每次渲染启动独立的无 GUI 子进程，300 秒超时。独占任务文件锁避免重启时两个
worker 同时写同一任务。输出先写同一文件系统中的隐藏临时目录，然后以 rename
提交整个任务目录。目录内有完成收据；数据库提交前崩溃可通过收据恢复而不重复输出。

重启将 Processing 退回 Queued，将未完整接收的 Receiving 置为 Failed。
重试复用原 UUID 和设置快照；用户修复原输出目录权限后即可重试。
修改设置只影响新任务，失败任务不会悄悄切换输出目录或尺寸。

PAPPL 的 end-job 回调等待后台实际生成文件，成功后才返回 true；失败返回 false，
IPP 报告 aborted。PAPPL 崩溃后由后台重启，接收中的任务会失败并要求重新提交。
父后台退出后，PAPPL 的父进程检查定时器结束适配器。
若暂存失败导致 manifest 本身无法写出，适配器另通过 IPC 回报失败；数据库完全
不可写时，保留日志并在空间恢复、后台重启后处理未完成的接收记录。

## 内部接口

| 类型 | 含义 |
|---|---|
| `PrintJobInput::Zpl` | 私有暂存文件，分辨率/缺省尺寸在任务设置快照中 |
| `PrintJobInput::RasterPages` | 最终页序，文件/尺寸/分辨率逐页保留 |
| `RenderedPage` | 灰度像素、宽高、物理尺寸及 X/Y 分辨率 |
| `JobRecord` | UUID、设置快照、输入、状态、输出、诊断 |
| `OutputReceipt` | 已原子提交的文件与页数，支持完成阶段恢复 |

`core::output` 是唯一的 Labelize 调用位置。ZPL 使用 `parse_with_options` 开启
逐标签尺寸、完整格式检查和 1000 页上限，缺省 `parse` 行为仍与 Labelize 原 API 兼容。
`^LL` 在首个 `^FS` 前生效，之后的设置影响下一标签；有效 `^PW`/`^LL` 优先于缺省纸张。

## 本地权限与生命周期

应用数据和 Unix IPC 父目录权限为 0700，socket/token 为 0600。
Windows Named Pipe 显式 DACL 仅授权当前用户 SID；请求另带私有目录中的随机 token。
RAW/IPP 仅监听 IPv4 loopback，不开放网络共享。RAW 协议本身无用户认证，
这是当前“本机单用户”范围的限制。

安装先尝试当前用户已有的打印管理权限，只有失败时才使用 macOS 系统授权或 Linux
PolicyKit；Windows 队列配置脚本按需提权。日常后台不提权。
已有同名但非本产品的打印队列/端口会被拒绝修改。

Windows 登录计划任务、macOS LaunchAgent、Linux 用户 systemd 提供下次登录启动。
关闭 GPUI 不停止后台；显式 `stop` 要求队列空闲。目录变化、端口冲突、渲染失败
都通过 IPC 在界面反馈。已完成安装后的端口冲突不会静默换端口，否则系统队列会失联；
需要用户解除占用并修复队列。

## 资源与保留

输入最多 64 MiB；栅格按规范化后且展开份数的总大小检查。最多 1000 页，
每页最多 16 Mi 像素、单边最多 32000 dots。尺寸和分辨率必须有限且为正数。
最多 4 个同时接收的 RAW 连接、16 个 IPC 处理连接、单个渲染 worker。
原始输入、失败诊断及完成历史首版保留在本地；不自动删除用户输出。
磁盘配额和历史清理策略留待后续，试用时应定期检查数据目录占用。

## 原始依据

- [PAPPL 栅格回调](https://www.msweet.org/pappl/pappl.html#the-raster-printing-callbacks)
- [PAPPL 1.4.12 源码](https://github.com/michaelrsweet/pappl/releases/tag/v1.4.12)
- [Zebra ZPL Programming Guide](https://www.zebra.com/content/dam/support-dam/en/documentation/unrestricted/guide/software/zpl-zbi2-pg-en.pdf)
- [OpenSSL 下载与版本](https://www.openssl-library.org/source/)
