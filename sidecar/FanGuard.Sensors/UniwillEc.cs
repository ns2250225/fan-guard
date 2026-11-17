// Uniwill / 同方（Tongfang）模具笔记本嵌入式控制器（EC）读取
//
// 机械革命、雷神、炫龙、TUXEDO、XMG 等品牌的同方/Uniwill 模具笔记本，风扇由 EC 控制，
// 没有 Super I/O 芯片，LibreHardwareMonitor 读不到风扇转速。
// 这类机器通过 ACPI WMI 接口 AcpiTest_MULong（GUID ABBC0F6F-8EA1-11D1-00A0-C90629100000）
// 的 GetSetULong 方法读写 EC RAM —— 厂商控制中心与 Linux uniwill-laptop / tuxedo 驱动使用的是同一接口。
//
// 输入参数 Data（UInt64，小端）：
//   byte0 = 地址低字节，byte1 = 地址高字节，byte2/3 = 写入数据，byte5 = 0x01 表示「读取」
// 返回值 Return（UInt32）：byte0 为读取到的数据。
//
// 本类只执行读取（byte5 恒为 0x01，数据字节恒为 0），绝不写入 EC。
// 该 WMI 类只允许管理员访问。

using System.Management;

namespace FanGuard.Sensors;

internal sealed class UniwillEc : IDisposable
{
    // EC RAM 地址（来源：Linux uniwill-laptop 驱动）
    private const ushort CpuTemp = 0x043E;
    private const ushort GpuTemp = 0x044F;
    private const ushort Fan1RpmHi = 0x0464;
    private const ushort Fan1RpmLo = 0x0465;
    private const ushort Fan2RpmHi = 0x046C;
    private const ushort Fan2RpmLo = 0x046D;
    private const ushort Fan1Pwm = 0x1804;
    private const ushort Fan2Pwm = 0x1809;
    private const double PwmMax = 200.0;

    private const ulong ReadFlag = 1UL << 40;

    private readonly ManagementObject _obj;
    private int _failures;

    private UniwillEc(ManagementObject obj)
    {
        _obj = obj;
    }

    /// <summary>状态：available / needAdmin / none</summary>
    public static string Probe(out UniwillEc? ec)
    {
        ec = null;
        try
        {
            using var searcher = new ManagementObjectSearcher(@"root\WMI", "SELECT * FROM AcpiTest_MULong");
            foreach (ManagementObject o in searcher.Get())
            {
                ec = new UniwillEc(o);
                // 试读一次 CPU 温度，确认接口可用
                if (ec.Read(CpuTemp) is null)
                {
                    ec.Dispose();
                    ec = null;
                    return "none";
                }

                return "available";
            }

            return "none";
        }
        catch (ManagementException ex) when (ex.ErrorCode == ManagementStatus.AccessDenied)
        {
            return "needAdmin";
        }
        catch (UnauthorizedAccessException)
        {
            return "needAdmin";
        }
        catch
        {
            return "none";
        }
    }

    private int? Read(ushort addr)
    {
        try
        {
            using var inParams = _obj.GetMethodParameters("GetSetULong");
            inParams["Data"] = (ulong)addr | ReadFlag;
            using var outParams = _obj.InvokeMethod("GetSetULong", inParams, null);
            var ret = outParams?["Return"];
            if (ret is null)
            {
                return null;
            }

            return (int)(Convert.ToUInt32(ret) & 0xFF);
        }
        catch
        {
            _failures++;
            return null;
        }
    }

    private double? Rpm(ushort hi, ushort lo)
    {
        var h = Read(hi);
        var l = Read(lo);
        if (h is null || l is null)
        {
            return null;
        }

        var rpm = (h.Value << 8) | l.Value;
        // 超出合理范围的值视为该机型不使用此寄存器
        return rpm is >= 0 and <= 12000 ? rpm : null;
    }

    private double? Pwm(ushort addr)
    {
        var v = Read(addr);
        return v is >= 0 and <= 200 ? Math.Round(v.Value / PwmMax * 100.0, 1) : null;
    }

    private double? Temp(ushort addr)
    {
        var v = Read(addr);
        return v is > 1 and < 120 ? v : null;
    }

    /// <summary>连续失败过多时停止读取</summary>
    public bool Broken => _failures > 50;

    public void Write(System.Text.Json.Utf8JsonWriter w)
    {
        var sensors = new List<(string id, string name, string type, double? value)>
        {
            ("fan/0", "CPU Fan", "Fan", Rpm(Fan1RpmHi, Fan1RpmLo)),
            ("fan/1", "GPU Fan", "Fan", Rpm(Fan2RpmHi, Fan2RpmLo)),
            ("control/0", "CPU Fan", "Control", Pwm(Fan1Pwm)),
            ("control/1", "GPU Fan", "Control", Pwm(Fan2Pwm)),
            ("temperature/0", "CPU (EC)", "Temperature", Temp(CpuTemp)),
            ("temperature/1", "GPU (EC)", "Temperature", Temp(GpuTemp)),
        };

        w.WriteStartObject();
        w.WriteString("id", "/ec/uniwill");
        w.WriteString("name", "嵌入式控制器 (Uniwill EC)");
        w.WriteString("type", "EmbeddedController");
        w.WriteNull("parent");
        w.WriteStartArray("sensors");
        foreach (var s in sensors)
        {
            w.WriteStartObject();
            w.WriteString("id", "/ec/uniwill/" + s.id);
            w.WriteString("name", s.name);
            w.WriteString("type", s.type);
            if (s.value.HasValue)
            {
                w.WriteNumber("value", s.value.Value);
            }
            else
            {
                w.WriteNull("value");
            }

            w.WriteNull("min");
            w.WriteNull("max");
            w.WriteEndObject();
        }

        w.WriteEndArray();
        w.WriteEndObject();
    }

    public void Dispose()
    {
        _obj.Dispose();
    }
}
