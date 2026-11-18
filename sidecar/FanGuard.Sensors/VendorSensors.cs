// 厂商传感器接口（只读）
//
// 很多笔记本的风扇由嵌入式控制器（EC）管理，没有 Super I/O 芯片，LibreHardwareMonitor 读不到转速。
// 这些机器的厂商控制软件通过各自的 ACPI WMI 接口读取风扇，本文件按品牌适配其中的「读取」方法：
//
//   品牌                 接口                                              来源
//   ASUS（ROG/TUF 等）   root\WMI AsusAtkWmi_WMNB.DSTS(设备 ID)             Linux asus-wmi 驱动、G-Helper
//   Lenovo Legion       root\WMI LENOVO_GAMEZONE_DATA.GetFan1Speed/2      Lenovo Legion Toolkit
//                       root\WMI LENOVO_FAN_METHOD.Fan_GetCurrentFanSpeed
//   Dell                root\dcim\sysman DCIM_NumericSensor（转速计）       Dell Command | Monitor（需安装）
//   同方 / Uniwill 模具   root\WMI AcpiTest_MULong.GetSetULong（读 EC）       Linux uniwill-laptop 驱动（见 UniwillEc.cs）
//
// 安全原则：只调用读取类方法（DSTS / Get*），绝不调用 DEVS / Set* 等写入方法；
// 品牌按 Win32_ComputerSystem 的制造商识别，避免把某个品牌的接口用到其他机器上。
// 除 Dell 外，这些 WMI 接口通常只允许管理员访问。

using System.Management;
using System.Text.Json;

namespace FanGuard.Sensors;

internal interface IVendorProvider : IDisposable
{
    string Id { get; }

    string Name { get; }

    bool Broken { get; }

    void Write(Utf8JsonWriter w);
}

internal readonly record struct VendorReading(string Id, string Name, string Type, double? Value);

internal static class VendorJson
{
    public static void WriteNode(Utf8JsonWriter w, string id, string name, IEnumerable<VendorReading> readings)
    {
        w.WriteStartObject();
        w.WriteString("id", "/vendor/" + id);
        w.WriteString("name", name);
        w.WriteString("type", "EmbeddedController");
        w.WriteNull("parent");
        w.WriteStartArray("sensors");
        foreach (var r in readings)
        {
            w.WriteStartObject();
            w.WriteString("id", $"/vendor/{id}/{r.Id}");
            w.WriteString("name", r.Name);
            w.WriteString("type", r.Type);
            if (r.Value.HasValue)
            {
                w.WriteNumber("value", Math.Round(r.Value.Value, 2));
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
}

/// <summary>对 WMI 实例方法的封装：参数名从方法定义中读取，避免硬编码导致的兼容问题</summary>
internal sealed class WmiMethod : IDisposable
{
    private readonly ManagementObject _obj;
    private readonly string _method;
    private readonly string? _inName;
    public int Failures;

    private WmiMethod(ManagementObject obj, string method, string? inName)
    {
        _obj = obj;
        _method = method;
        _inName = inName;
    }

    /// <summary>查找 WMI 类的第一个实例上的方法；访问被拒绝时抛出 UnauthorizedAccessException</summary>
    public static WmiMethod? Find(string ns, string cls, string method)
    {
        try
        {
            using var searcher = new ManagementObjectSearcher(ns, $"SELECT * FROM {cls}");
            foreach (ManagementObject o in searcher.Get())
            {
                string? inName = null;
                try
                {
                    using var p = o.GetMethodParameters(method);
                    foreach (PropertyData pd in p.Properties)
                    {
                        inName = pd.Name;
                        break;
                    }
                }
                catch (ManagementException ex) when (ex.ErrorCode == ManagementStatus.MethodNotImplemented || ex.ErrorCode == ManagementStatus.NotFound || ex.ErrorCode == ManagementStatus.InvalidMethod)
                {
                    o.Dispose();
                    return null;
                }

                return new WmiMethod(o, method, inName);
            }
        }
        catch (ManagementException ex) when (ex.ErrorCode == ManagementStatus.AccessDenied)
        {
            throw new UnauthorizedAccessException();
        }
        catch (ManagementException)
        {
            // 类不存在（InvalidClass / InvalidNamespace 等）
        }

        return null;
    }

    /// <summary>调用方法并返回第一个数值型输出参数</summary>
    public uint? Call(uint? arg = null)
    {
        try
        {
            ManagementBaseObject? inParams = null;
            if (arg.HasValue && _inName != null)
            {
                inParams = _obj.GetMethodParameters(_method);
                inParams[_inName] = arg.Value;
            }

            using var outParams = _obj.InvokeMethod(_method, inParams, null);
            inParams?.Dispose();
            if (outParams == null)
            {
                return null;
            }

            foreach (PropertyData pd in outParams.Properties)
            {
                if (pd.Name == "ReturnValue" || pd.Value == null)
                {
                    continue;
                }

                return Convert.ToUInt32(pd.Value);
            }

            return null;
        }
        catch
        {
            Failures++;
            return null;
        }
    }

    public void Dispose() => _obj.Dispose();
}

/// <summary>ASUS ATK WMI：DSTS 读取设备状态。风扇设备返回值低 16 位为转速（单位 100 RPM），bit16 为「设备存在」</summary>
internal sealed class AsusProvider : IVendorProvider
{
    private const uint CpuFan = 0x00110013;
    private const uint GpuFan = 0x00110014;
    private const uint MidFan = 0x00110031;
    private const uint PresenceBit = 0x00010000;

    private readonly WmiMethod _dsts;
    private readonly List<(uint dev, string name)> _fans = new();

    private AsusProvider(WmiMethod dsts)
    {
        _dsts = dsts;
    }

    public string Id => "asus";

    public string Name => "ASUS ATK WMI";

    public bool Broken => _dsts.Failures > 50;

    private double? Rpm(uint dev)
    {
        var v = _dsts.Call(dev);
        if (v is null || (v.Value & PresenceBit) == 0 || v.Value == 0xFFFFFFFE)
        {
            return null;
        }

        var rpm = (v.Value & 0xFFFF) * 100.0;
        return rpm <= 12000 ? rpm : null;
    }

    public static IVendorProvider? Probe()
    {
        var m = WmiMethod.Find(@"root\WMI", "AsusAtkWmi_WMNB", "DSTS");
        if (m == null)
        {
            return null;
        }

        var p = new AsusProvider(m);
        foreach (var (dev, name) in new[] { (CpuFan, "CPU Fan"), (GpuFan, "GPU Fan"), (MidFan, "Mid Fan") })
        {
            if (p.Rpm(dev) != null)
            {
                p._fans.Add((dev, name));
            }
        }

        if (p._fans.Count == 0)
        {
            p.Dispose();
            return null;
        }

        return p;
    }

    public void Write(Utf8JsonWriter w)
    {
        VendorJson.WriteNode(w, Id, Name, _fans.Select(f => new VendorReading($"fan/{f.dev:x}", f.name, "Fan", Rpm(f.dev))));
    }

    public void Dispose() => _dsts.Dispose();
}

/// <summary>Lenovo Legion：优先使用 LENOVO_GAMEZONE_DATA.GetFanXSpeed，新机型使用 LENOVO_FAN_METHOD</summary>
internal sealed class LenovoProvider : IVendorProvider
{
    private readonly List<(WmiMethod m, uint? arg, string name)> _fans;

    private LenovoProvider(List<(WmiMethod m, uint? arg, string name)> fans)
    {
        _fans = fans;
    }

    public string Id => "lenovo";

    public string Name => "Lenovo Legion WMI";

    public bool Broken => _fans.All(f => f.m.Failures > 50);

    private static double? Rpm(WmiMethod m, uint? arg)
    {
        var v = m.Call(arg);
        return v is > 0 and <= 12000 ? v : v == 0 ? 0 : null;
    }

    public static IVendorProvider? Probe()
    {
        var fans = new List<(WmiMethod, uint?, string)>();
        var f1 = WmiMethod.Find(@"root\WMI", "LENOVO_GAMEZONE_DATA", "GetFan1Speed");
        var f2 = WmiMethod.Find(@"root\WMI", "LENOVO_GAMEZONE_DATA", "GetFan2Speed");
        if (f1 != null && Rpm(f1, null) != null)
        {
            fans.Add((f1, null, "CPU Fan"));
        }
        else
        {
            f1?.Dispose();
        }

        if (f2 != null && Rpm(f2, null) != null)
        {
            fans.Add((f2, null, "GPU Fan"));
        }
        else
        {
            f2?.Dispose();
        }

        if (fans.Count == 0)
        {
            // 新款 Legion：风扇编号 1 = CPU，2 = GPU
            foreach (var (id, name) in new[] { (1u, "CPU Fan"), (2u, "GPU Fan") })
            {
                var m = WmiMethod.Find(@"root\WMI", "LENOVO_FAN_METHOD", "Fan_GetCurrentFanSpeed");
                if (m != null && Rpm(m, id) != null)
                {
                    fans.Add((m, id, name));
                }
                else
                {
                    m?.Dispose();
                }
            }
        }

        return fans.Count > 0 ? new LenovoProvider(fans) : null;
    }

    public void Write(Utf8JsonWriter w)
    {
        VendorJson.WriteNode(w, Id, Name, _fans.Select((f, i) => new VendorReading($"fan/{i}", f.name, "Fan", Rpm(f.m, f.arg))));
    }

    public void Dispose()
    {
        foreach (var f in _fans)
        {
            f.m.Dispose();
        }
    }
}

/// <summary>Dell Command | Monitor：DCIM_NumericSensor 中 SensorType = 5（转速计）的传感器</summary>
internal sealed class DellProvider : IVendorProvider
{
    private int _failures;

    public string Id => "dell";

    public string Name => "Dell Command | Monitor";

    public bool Broken => _failures > 50;

    private List<VendorReading>? Read()
    {
        try
        {
            using var searcher = new ManagementObjectSearcher(@"root\dcim\sysman", "SELECT ElementName, CurrentReading, UnitModifier FROM DCIM_NumericSensor WHERE SensorType = 5");
            var list = new List<VendorReading>();
            var i = 0;
            foreach (ManagementObject o in searcher.Get())
            {
                using (o)
                {
                    var name = o["ElementName"]?.ToString() ?? $"Fan {i + 1}";
                    var reading = o["CurrentReading"] is { } r ? Convert.ToDouble(r) : (double?)null;
                    var mod = o["UnitModifier"] is { } m ? Convert.ToInt32(m) : 0;
                    var rpm = reading * Math.Pow(10, mod);
                    list.Add(new VendorReading($"fan/{i}", name, "Fan", rpm is >= 0 and <= 20000 ? rpm : null));
                    i++;
                }
            }

            return list;
        }
        catch (ManagementException ex) when (ex.ErrorCode == ManagementStatus.AccessDenied)
        {
            throw new UnauthorizedAccessException();
        }
        catch
        {
            _failures++;
            return null;
        }
    }

    public static IVendorProvider? Probe()
    {
        var p = new DellProvider();
        var r = p.Read();
        return r is { Count: > 0 } ? p : null;
    }

    public void Write(Utf8JsonWriter w)
    {
        VendorJson.WriteNode(w, Id, Name, Read() ?? new List<VendorReading>());
    }

    public void Dispose()
    {
    }
}

internal static class VendorSensors
{
    /// <summary>
    /// 同方 / Uniwill 模具：GetSetULong 是读写共用的接口，同样的 WMI 类在蓝天（Clevo）等模具上含义不同，
    /// 因此只对已知使用该模具的品牌或主板型号启用。
    /// </summary>
    private static readonly string[] UniwillVendors =
    {
        "mechrevo", "thunderobot", "tuxedo", "schenker", "xmg", "eluktronics", "tongfang", "uniwill",
    };

    private static readonly System.Text.RegularExpressions.Regex UniwillBoard = new(@"^(GM|GK|GX|PH|PF)\d", System.Text.RegularExpressions.RegexOptions.IgnoreCase);

    public static (string manufacturer, string model, string board) SystemInfo()
    {
        string m = "", model = "", board = "";
        try
        {
            using var cs = new ManagementObjectSearcher(@"root\CIMV2", "SELECT Manufacturer, Model FROM Win32_ComputerSystem");
            foreach (ManagementObject o in cs.Get())
            {
                using (o)
                {
                    m = o["Manufacturer"]?.ToString()?.Trim() ?? "";
                    model = o["Model"]?.ToString()?.Trim() ?? "";
                }
            }

            using var bb = new ManagementObjectSearcher(@"root\CIMV2", "SELECT Product FROM Win32_BaseBoard");
            foreach (ManagementObject o in bb.Get())
            {
                using (o)
                {
                    board = o["Product"]?.ToString()?.Trim() ?? "";
                }
            }
        }
        catch
        {
            // ignore
        }

        return (m, model, board);
    }

    /// <summary>
    /// 根据品牌选择厂商接口。返回 (提供者, 品牌标识, 显示名称, 状态)；
    /// 状态：available / needAdmin / none（未找到可用接口）/ unsupported（尚未适配的品牌）
    /// </summary>
    public static (IVendorProvider? provider, string id, string name, string status) Detect(string manufacturer, string board)
    {
        var m = manufacturer.ToLowerInvariant();
        (string id, string name, Func<IVendorProvider?> probe)? choice = null;

        if (m.Contains("asus"))
        {
            choice = ("asus", "ASUS ATK WMI", AsusProvider.Probe);
        }
        else if (m.Contains("lenovo"))
        {
            choice = ("lenovo", "Lenovo Legion WMI", LenovoProvider.Probe);
        }
        else if (m.Contains("dell") || m.Contains("alienware"))
        {
            choice = ("dell", "Dell Command | Monitor", DellProvider.Probe);
        }
        else if (UniwillVendors.Any(m.Contains) || UniwillBoard.IsMatch(board))
        {
            choice = ("uniwill", "同方 / Uniwill EC", () =>
            {
                var s = UniwillEc.Probe(out var ec);
                if (s == "needAdmin")
                {
                    throw new UnauthorizedAccessException();
                }

                return ec;
            });
        }

        if (choice == null)
        {
            return (null, "none", "", "unsupported");
        }

        try
        {
            var p = choice.Value.probe();
            return (p, choice.Value.id, choice.Value.name, p != null ? "available" : "none");
        }
        catch (UnauthorizedAccessException)
        {
            return (null, choice.Value.id, choice.Value.name, "needAdmin");
        }
        catch
        {
            return (null, choice.Value.id, choice.Value.name, "none");
        }
    }
}
