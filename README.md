# FanGuard 风扇管家

Windows 10 / 11 桌面硬件监控与进程诊断工具：实时监控 CPU / GPU / 内存 / 磁盘 / 温度 / 风扇转速，定位高负载进程，智能分析风扇狂转的原因，并安全地结束异常进程。

> 电脑没有运行大型软件，风扇却突然狂转？FanGuard 不仅告诉你是哪个进程占用了资源，还会告诉你为什么电脑发热、为什么风扇狂转，以及如何安全地解决问题。

## 功能特性

- **仪表盘**：CPU / GPU / 内存 / 磁盘 / 风扇状态卡片，最近 60 秒实时负载曲线，异常时顶部提示并可直接跳转排查。
- **进程管理**：全量进程列表（CPU / 内存 / GPU / 显存多列排序、按名称或 PID 搜索、高负载排行），进程详情（文件路径、数字签名、线程、父子进程），支持结束进程 / 强制结束 / 结束进程树。
- **智能诊断**：基于时间窗口统计（而非瞬时值）计算每个进程的「异常关联分数」，检测 CPU/GPU 持续高负载、高温、风扇高速旋转但负载低等异常，给出排查对象与优化建议。
- **硬件监控**：CPU / GPU / 内存 / 磁盘 / 温度 / 风扇转速全覆盖，硬件不支持时明确显示「暂不支持读取」，绝不显示虚构数据。
- **历史记录**：本地 SQLite 存储 CPU / GPU / 温度 / 风扇历史曲线与异常事件，支持 CSV 导出。
- **系统托盘**：最小化到托盘后台运行，支持开机自启动。

## 技术栈

| 模块 | 技术 |
|---|---|
| 桌面框架 | Tauri 2 |
| 前端 | Vue 3 + TypeScript + Tailwind CSS 4（shadcn 风格组件）+ ECharts + Vue Router |
| 后端 | Rust |
| 进程管理 | Windows API（NtQuerySystemInformation、OpenProcess、TerminateProcess 等） |
| GPU 进程占用 | Windows PDH 性能计数器（GPU Engine / GPU Process Memory） |
| 硬件传感器 | LibreHardwareMonitor（.NET 8 Sidecar，随主程序打包）+ HWiNFO 共享内存，NVIDIA 显卡额外支持 NVML 回退 |
| 数据库 | SQLite（rusqlite，bundled 内置） |

## 目录结构

```
src/                  Vue 前端
  views/              仪表盘 / 进程管理 / 智能诊断 / 硬件监控 / 历史记录 / 系统设置
  components/         通用组件、进程详情、结束进程确认流程、图表
src-tauri/src/        Rust 核心
  process.rs          进程枚举、CPU 占用计算、进程分类与关键进程保护
  process_ops.rs      结束进程 / 强制结束 / 结束进程树、线程、文件信息、数字签名
  pdh.rs              GPU、磁盘等性能计数器
  hardware.rs         CPU / 内存 / 显卡适配器信息
  sensors.rs          LibreHardwareMonitor Sidecar 守护与解析、NVML 回退
  hwinfo.rs           HWiNFO 共享内存读取
  hwinfo_manager.rs   HWiNFO 查找 / 导入 / 配置 / 启动 / 续期
  diagnosis.rs        智能诊断引擎（时间窗口统计、异常关联评分、异常检测、建议）
  monitor.rs          监控主循环与实时快照
  db.rs               SQLite 历史记录与 CSV 导出
  settings.rs         用户设置
  tray.rs             系统托盘
sidecar/FanGuard.Sensors/   LibreHardwareMonitor 传感器 Sidecar（C# / .NET 8）
scripts/              Sidecar / 独立 exe 构建脚本、图标生成
```

## 环境要求

- Node.js 18+
- Rust（MSVC 工具链，x86_64-pc-windows-msvc）
- .NET 8 SDK（构建传感器 Sidecar）
- WebView2 运行时（Windows 11 自带）

## 构建

```powershell
npm install
npm run sidecar        # 构建 LibreHardwareMonitor 传感器 Sidecar，输出到 src-tauri/binaries
npm run tauri dev      # 开发运行
npm run tauri build    # 打包 Windows 安装程序（NSIS / MSI）
```

> Sidecar 必须先构建，Tauri 打包时会将 `src-tauri/binaries/fanguard-sensors-x86_64-pc-windows-msvc.exe` 作为 externalBin 一同打包。

## 关于传感器与权限

- FanGuard 默认 **不申请管理员权限**。进程、CPU、内存、磁盘、GPU 使用率在普通权限下即可读取。
- CPU 温度 / 功耗、主板风扇转速通常需要以管理员身份运行；LibreHardwareMonitor 0.9.5 起使用 PawnIO 驱动读取底层传感器，如果系统未安装 PawnIO，这类数据可能仍不可用。
- 硬件不支持读取时界面显示「暂不支持读取」，不会显示虚构数据，诊断仍会基于温度与负载进行。
- 软件内提供「以管理员身份重启」按钮，仅在用户主动点击时请求 UAC 授权。

## 厂商风扇接口（按品牌适配）

很多笔记本的风扇由嵌入式控制器（EC）管理，LibreHardwareMonitor 与 HWiNFO 都读不到转速。传感器服务按 `Win32_ComputerSystem` 的制造商识别品牌，调用厂商控制软件使用的 ACPI WMI 读取接口：

| 品牌 | 接口 | 参考来源 | 验证状态 |
|---|---|---|---|
| 同方 / Uniwill 模具（机械革命、雷神、TUXEDO、XMG 等） | `AcpiTest_MULong.GetSetULong`（读 EC） | Linux uniwill-laptop 驱动 | 已在机械革命 GM5AGEE 上确认接口存在 |
| ASUS（ROG / TUF 等） | `AsusAtkWmi_WMNB.DSTS` | Linux asus-wmi、G-Helper | 未在真机验证 |
| Lenovo Legion | `LENOVO_GAMEZONE_DATA.GetFanXSpeed`、`LENOVO_FAN_METHOD.Fan_GetCurrentFanSpeed` | Lenovo Legion Toolkit | 未在真机验证 |
| Dell | `root\dcim\sysman` `DCIM_NumericSensor`（转速计） | Dell Command \| Monitor（需安装） | 未在真机验证 |

- 只调用读取方法（DSTS / Get*），不调用任何写入方法；
- 同方 / Uniwill 接口读写共用，且同名 WMI 类在蓝天（Clevo）模具上含义不同，因此只对已知品牌或主板型号（GM/GK/GX/PH/PF 开头）启用；
- 除 Dell 外这些接口只允许管理员访问，需以管理员身份运行 FanGuard；
- 未适配：HP（OMEN BIOS 接口为读写共用的命令协议）、MSI、Gigabyte 笔记本、ThinkPad。

## PawnIO 驱动

LibreHardwareMonitor 0.9.5 起通过 PawnIO 驱动读取 CPU 温度 / 功耗与台式机主板风扇。FanGuard 会检测该驱动（与 LibreHardwareMonitor 相同，读取 `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO`），未安装或版本较旧时在「硬件监控」页与「系统设置」中提供一键安装：

- **内置安装程序**：官方 `PawnIO_setup.exe`（安装程序声明 "This installer can be redistributed unmodified"），构建脚本缺失时会自动下载；
- **安全安装**：运行前校验 namazso 数字签名，通过一次 UAC 授权执行 `-install -silent`，完成后自动重新加载传感器服务；
- **权限**：驱动安装后仍需以管理员身份运行 FanGuard，LibreHardwareMonitor 才能访问它。

## HWiNFO 内置与自动集成

构建时若项目根目录（或 `src-tauri/binaries/`）存在 `hwinfo.zip`（HWiNFO 便携版），会提取其中的 `HWiNFO64.exe` 内置到 FanGuard.exe：

- **自动释放与配置**：需要时释放到 `%LOCALAPPDATA%\FanGuard\hwinfo`，写入 `HWiNFO64.INI`（仅传感器、开启共享内存、启动最小化、不显示欢迎窗口）；
- **自动启动**：LibreHardwareMonitor 读不到 CPU 温度或风扇时启动；HWiNFO 需要管理员权限，FanGuard 未提权时会出现一次 UAC 确认；
- **安全校验**：每次启动 HWiNFO 前校验 REALiX 数字签名；
- **自动续期**：免费版共享内存 12 小时后失效，检测到后自动重启 HWiNFO；
- **未内置时**：仍支持自动导入「下载」文件夹或 FanGuard 所在目录中的便携版。

> 许可提醒：HWiNFO 是 REALiX 的专有软件，免费版仅限非商业使用。将内置 HWiNFO 的 FanGuard 分发给他人之前，请先确认 REALiX 的再分发许可。

## 安全与隐私设计

- 关键系统进程（System、csrss、wininit、winlogon、services、lsass、svchost、dwm 等）禁止结束；伪装在系统目录之外的同名进程不享受保护。
- 结束前以 PID + 创建时间校验进程身份，避免 PID 重用误杀。
- 「结束进程」优先向窗口发送关闭请求（正常结束）；强制结束需确认；结束进程树需二次确认并跳过受保护的子进程。
- 不提供无确认的批量强制结束，不自动结束任何进程，不修改风扇转速。
- 所有数据和操作记录仅保存在本地（`%APPDATA%\com.fanguard.app`），不上传任何进程信息与硬件数据。

## 许可证

本项目暂未附带开源许可证，如需使用或分发请先联系作者。
