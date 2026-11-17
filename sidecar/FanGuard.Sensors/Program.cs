// FanGuard 硬件传感器 Sidecar
//
// 基于 LibreHardwareMonitorLib 采集 CPU / GPU / 主板 / 存储 / 嵌入式控制器的温度、功耗、频率、风扇转速。
// 进程常驻，通过 stdout 按行输出 JSON，通过 stdin 接收控制命令，避免每次刷新重新启动监控服务。
//
// 输出协议（每行一个 JSON 对象）：
//   {"type":"hello","admin":true,"version":"1.0.0","lhm":"0.9.6"}
//   {"type":"data","ts":1700000000000,"hardware":[{"id":"/amdcpu/0","name":"...","type":"Cpu","parent":null,"sensors":[{"id":"...","name":"...","type":"Temperature","value":55.1,"min":40.0,"max":80.0}]}]}
//   {"type":"error","message":"..."}
//
// 输入命令（每行一个）：
//   interval <毫秒>   修改采集间隔
//   pause            暂停采集
//   resume           恢复采集
//   quit             退出
//
// 启动参数：--interval <毫秒> --parent <父进程 PID>（父进程退出后自动退出，避免孤儿进程）

using System.Diagnostics;
using System.Security.Principal;
using System.Text;
using System.Text.Json;
using LibreHardwareMonitor.Hardware;

namespace FanGuard.Sensors;

internal sealed class UpdateVisitor : IVisitor
{
    public void VisitComputer(IComputer computer) => computer.Traverse(this);

    public void VisitHardware(IHardware hardware)
    {
        try
        {
            hardware.Update();
        }
        catch
        {
            // 个别硬件读取失败不影响其他硬件
        }

        foreach (var sub in hardware.SubHardware)
        {
            sub.Accept(this);
        }
    }

    public void VisitSensor(ISensor sensor) { }

    public void VisitParameter(IParameter parameter) { }
}

internal static class Program
{
    private static volatile int _intervalMs = 2000;
    private static volatile bool _paused;
    private static volatile bool _quit;
    private static readonly ManualResetEventSlim Wake = new(false);
    private static readonly object OutLock = new();
    private static Stream _stdout = Stream.Null;
    private static UniwillEc? _ec;
    private static string _ecStatus = "none";

    private static int Main(string[] args)
    {
        int parentPid = 0;
        for (int i = 0; i < args.Length; i++)
        {
            if (args[i] == "--interval" && i + 1 < args.Length && int.TryParse(args[i + 1], out var ms))
            {
                _intervalMs = Math.Clamp(ms, 250, 60000);
                i++;
            }
            else if (args[i] == "--parent" && i + 1 < args.Length && int.TryParse(args[i + 1], out var pid))
            {
                parentPid = pid;
                i++;
            }
        }

        _stdout = Console.OpenStandardOutput();

        bool admin = IsAdministrator();
        // 同方 / Uniwill 模具笔记本：通过 EC 读取风扇转速（需要管理员权限）
        _ecStatus = UniwillEc.Probe(out _ec);
        WriteHello(admin);

        StartStdinReader();
        if (parentPid > 0)
        {
            StartParentWatcher(parentPid);
        }

        var computer = new Computer
        {
            IsCpuEnabled = true,
            IsGpuEnabled = true,
            IsMemoryEnabled = true,
            IsMotherboardEnabled = true,
            IsControllerEnabled = true,
            IsStorageEnabled = true,
            IsNetworkEnabled = false,
            IsBatteryEnabled = true,
            IsPsuEnabled = true,
        };

        try
        {
            computer.Open();
        }
        catch (Exception ex)
        {
            WriteError("无法初始化 LibreHardwareMonitor: " + ex.Message);
        }

        var visitor = new UpdateVisitor();

        while (!_quit)
        {
            if (!_paused)
            {
                try
                {
                    computer.Accept(visitor);
                    WriteData(computer);
                }
                catch (Exception ex)
                {
                    WriteError("采集失败: " + ex.Message);
                }
            }

            Wake.Wait(_paused ? 1000 : _intervalMs);
            Wake.Reset();
        }

        try
        {
            computer.Close();
        }
        catch
        {
            // ignore
        }

        return 0;
    }

    private static bool IsAdministrator()
    {
        try
        {
            using var identity = WindowsIdentity.GetCurrent();
            return new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
        }
        catch
        {
            return false;
        }
    }

    private static void StartStdinReader()
    {
        var t = new Thread(() =>
        {
            try
            {
                using var reader = new StreamReader(Console.OpenStandardInput(), Encoding.UTF8);
                string? line;
                while ((line = reader.ReadLine()) != null)
                {
                    line = line.Trim();
                    if (line.Length == 0) continue;
                    var parts = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
                    switch (parts[0])
                    {
                        case "interval":
                            if (parts.Length > 1 && int.TryParse(parts[1], out var ms))
                            {
                                _intervalMs = Math.Clamp(ms, 250, 60000);
                                Wake.Set();
                            }
                            break;
                        case "pause":
                            _paused = true;
                            break;
                        case "resume":
                            _paused = false;
                            Wake.Set();
                            break;
                        case "quit":
                            _quit = true;
                            Wake.Set();
                            return;
                    }
                }
            }
            catch
            {
                // stdin 关闭
            }

            // stdin 关闭意味着父进程已经退出
            _quit = true;
            Wake.Set();
        })
        {
            IsBackground = true,
            Name = "stdin-reader",
        };
        t.Start();
    }

    private static void StartParentWatcher(int parentPid)
    {
        var t = new Thread(() =>
        {
            try
            {
                using var parent = Process.GetProcessById(parentPid);
                parent.WaitForExit();
            }
            catch
            {
                // 父进程不存在
            }

            _quit = true;
            Wake.Set();
        })
        {
            IsBackground = true,
            Name = "parent-watcher",
        };
        t.Start();
    }

    private static void WriteLine(Action<Utf8JsonWriter> build)
    {
        lock (OutLock)
        {
            try
            {
                using var ms = new MemoryStream(16 * 1024);
                using (var w = new Utf8JsonWriter(ms))
                {
                    build(w);
                }

                ms.WriteByte((byte)'\n');
                ms.Position = 0;
                ms.CopyTo(_stdout);
                _stdout.Flush();
            }
            catch (IOException)
            {
                _quit = true;
                Wake.Set();
            }
        }
    }

    private static void WriteHello(bool admin)
    {
        WriteLine(w =>
        {
            w.WriteStartObject();
            w.WriteString("type", "hello");
            w.WriteBoolean("admin", admin);
            w.WriteString("version", "1.0.0");
            w.WriteString("lhm", typeof(Computer).Assembly.GetName().Version?.ToString() ?? "unknown");
            w.WriteString("ec", _ecStatus);
            w.WriteEndObject();
        });
    }

    private static void WriteError(string message)
    {
        WriteLine(w =>
        {
            w.WriteStartObject();
            w.WriteString("type", "error");
            w.WriteString("message", message);
            w.WriteEndObject();
        });
    }

    private static void WriteData(Computer computer)
    {
        WriteLine(w =>
        {
            w.WriteStartObject();
            w.WriteString("type", "data");
            w.WriteNumber("ts", DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
            w.WriteStartArray("hardware");
            foreach (var hw in computer.Hardware)
            {
                WriteHardware(w, hw, null);
            }

            if (_ec != null)
            {
                if (_ec.Broken)
                {
                    _ec.Dispose();
                    _ec = null;
                }
                else
                {
                    _ec.Write(w);
                }
            }

            w.WriteEndArray();
            w.WriteEndObject();
        });
    }

    private static void WriteHardware(Utf8JsonWriter w, IHardware hw, IHardware? parent)
    {
        w.WriteStartObject();
        w.WriteString("id", hw.Identifier.ToString());
        w.WriteString("name", hw.Name);
        w.WriteString("type", hw.HardwareType.ToString());
        if (parent != null)
        {
            w.WriteString("parent", parent.Identifier.ToString());
            w.WriteString("parentType", parent.HardwareType.ToString());
            w.WriteString("parentName", parent.Name);
        }
        else
        {
            w.WriteNull("parent");
        }

        w.WriteStartArray("sensors");
        foreach (var s in hw.Sensors)
        {
            w.WriteStartObject();
            w.WriteString("id", s.Identifier.ToString());
            w.WriteString("name", s.Name);
            w.WriteString("type", s.SensorType.ToString());
            WriteNullableFloat(w, "value", s.Value);
            WriteNullableFloat(w, "min", s.Min);
            WriteNullableFloat(w, "max", s.Max);
            w.WriteEndObject();
        }

        w.WriteEndArray();
        w.WriteEndObject();

        foreach (var sub in hw.SubHardware)
        {
            WriteHardware(w, sub, hw);
        }
    }

    private static void WriteNullableFloat(Utf8JsonWriter w, string name, float? value)
    {
        if (value.HasValue && !float.IsNaN(value.Value) && !float.IsInfinity(value.Value))
        {
            w.WriteNumber(name, Math.Round(value.Value, 2));
        }
        else
        {
            w.WriteNull(name);
        }
    }
}
