using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin.Hosting;

/// <summary>Conversions between the idiomatic SDK types and the <c>imt.plugin.v1</c> messages.</summary>
internal static class ProtoMapping
{
    public static Wire.Manifest ToProto(PluginDefinition d)
    {
        var m = d.Manifest;
        var pb = new Wire.Manifest
        {
            Id = m.Id,
            Name = m.Name,
            Version = m.Version,
            ContractMajor = Contract.Major,
            ContractMinor = Contract.Minor,
        };
        pb.Requires.AddRange(m.Requires.Select(r => new Wire.Dependency { Id = r.Id, Req = r.Requirement, Optional = r.Optional }));
        pb.Provides.AddRange(m.Provides.Select(p => new Wire.ServiceDecl { Id = p.Id, Version = p.Version }));
        pb.Permissions.AddRange(m.Permissions.Select(ToProto));
        pb.Capabilities.AddRange(Capabilities(d));
        return pb;
    }

    public static IEnumerable<Wire.Capability> Capabilities(PluginDefinition d)
    {
        foreach (var i in d.Importers)
        {
            yield return new Wire.Capability
            {
                Importer = new Wire.ImporterCap { Id = i.Id, Binary = i.Binary, Tail = !i.Binary && i is ITailImporter, Multiline = i.Multiline },
            };
        }

        foreach (var t in d.Transforms)
        {
            yield return new Wire.Capability { Transform = new Wire.TransformCap { Id = t.Id } };
        }

        foreach (var e in d.Exporters)
        {
            yield return new Wire.Capability { Exporter = new Wire.ExporterCap { Id = e.Id, Name = e.Name, Extension = e.Extension } };
        }

        if (d.MarkerKinds.Count > 0)
        {
            yield return new Wire.Capability { MarkerKinds = new Wire.MarkerKindsCap() };
        }

        foreach (var a in d.Annotators)
        {
            var cap = new Wire.AnnotatorCap { Id = a.Id, Limit = (uint)Math.Max(0, a.Limit) };
            cap.Interest.AddRange(a.Interest.Select(ToProto));
            yield return new Wire.Capability { Annotator = cap };
        }

        if (d.Settings is not null)
        {
            yield return new Wire.Capability { Settings = new Wire.SettingsCap() };
        }

        foreach (var s in d.Services)
        {
            yield return new Wire.Capability { Service = new Wire.ServiceCap { Id = s.ServiceId, Version = s.ServiceVersion } };
        }

        if (d.EventKinds.Count > 0)
        {
            var events = new Wire.EventsCap();
            events.Kinds.AddRange(d.EventKinds);
            yield return new Wire.Capability { Events = events };
        }
    }

    public static Wire.Permission ToProto(PermissionRequest p) => new()
    {
        Kind = p.Permission.Kind == PermissionKind.Network ? Wire.PermissionKind.Network : Wire.PermissionKind.ReadOpenedFiles,
        Target = p.Permission.Kind == PermissionKind.Network ? p.Permission.Target : string.Empty,
        Reason = p.Reason,
    };

    public static Permission? FromProto(Wire.Permission p) => p.Kind switch
    {
        Wire.PermissionKind.ReadOpenedFiles => Permission.ReadOpenedFiles,
        Wire.PermissionKind.Network => Permission.Network(p.Target),
        _ => null,
    };

    public static Wire.Condition ToProto(Condition c) => new()
    {
        Field = c.Field,
        Op = c.Op,
        ValueJson = c.Value?.ToJsonString() ?? string.Empty,
    };

    public static Wire.MarkerKind ToProto(MarkerKind k)
    {
        var pb = new Wire.MarkerKind
        {
            Kind = k.Kind,
            Label = k.Label,
            Description = k.Description ?? string.Empty,
            Color = k.Color ?? string.Empty,
            Icon = k.Icon ?? string.Empty,
            Badge = k.Badge ?? string.Empty,
            Shape = k.Shape ?? string.Empty,
            Role = k.Role switch
            {
                MarkerRole.Start => Wire.MarkerRole.RoleStart,
                MarkerRole.Stop => Wire.MarkerRole.RoleStop,
                MarkerRole.Crash => Wire.MarkerRole.RoleCrash,
                _ => Wire.MarkerRole.RoleNone,
            },
        };
        pb.When.AddRange(k.When.Select(ToProto));
        return pb;
    }

    public static Wire.Annotation ToProto(Annotation a) => new()
    {
        Kind = a.Kind,
        From = a.From,
        To = a.To,
        Instance = a.Instance ?? string.Empty,
        OnField = a.OnField ?? string.Empty,
        OnValue = a.OnField is null ? string.Empty : a.OnValue ?? string.Empty,
        Label = a.Label,
        Severity = a.Severity switch
        {
            Severity.Critical => Wire.Severity.Critical,
            Severity.Warning => Wire.Severity.Warning,
            _ => Wire.Severity.Info,
        },
        EvidenceQueryJson = a.EvidenceQuery?.ToJsonString() ?? string.Empty,
    };

    public static Wire.SettingsSection ToProto(SettingsSection s)
    {
        var pb = new Wire.SettingsSection { Title = s.Title, Description = s.Description };
        pb.Fields.AddRange(s.Fields.Select(f => new Wire.SettingField
        {
            Key = f.Key,
            Label = f.Label,
            Description = f.Description,
            Kind = ToProto(f.Kind),
            DefaultJson = f.Default?.ToJsonString() ?? "null",
            RestartRequired = f.RestartRequired,
            Advanced = f.Advanced,
        }));
        return pb;
    }

    public static Wire.FieldKind ToProto(SettingKind kind)
    {
        var k = new Wire.FieldKind();
        switch (kind)
        {
            case BoolSetting:
                k.Bool = new Wire.BoolKind();
                break;
            case IntSetting i:
                k.Int = new Wire.IntKind { Min = i.Min, Max = i.Max, Step = i.Step, Unit = i.Unit ?? string.Empty };
                break;
            case FloatSetting f:
                k.Float = new Wire.FloatKind { Min = f.Min, Max = f.Max, Step = f.Step, Unit = f.Unit ?? string.Empty };
                break;
            case TextSetting t:
                k.Text = new Wire.TextKind { MaxLen = (ulong)t.MaxLen, Pattern = t.Pattern ?? string.Empty };
                break;
            case EnumSetting e:
                k.Enum = new Wire.EnumKind();
                k.Enum.Options.AddRange(e.Options.Select(o => new Wire.EnumOption { Value = o.Value, Label = o.Label }));
                break;
            case PathSetting p:
                k.Path = new Wire.PathKind { Dir = p.Dir };
                break;
            case DurationSetting d:
                k.Duration = new Wire.DurationKind { MinMs = d.MinMs, MaxMs = d.MaxMs };
                break;
            case SizeSetting z:
                k.Size = new Wire.SizeKind { MinBytes = z.MinBytes, MaxBytes = z.MaxBytes };
                break;
            case StringListSetting l:
                k.StringList = new Wire.StringListKind { MaxItems = (ulong)l.MaxItems };
                break;
        }

        return k;
    }

    public static ImportContext Context(Wire.ImportContext? ctx, bool mayReadFiles)
    {
        if (ctx is null)
        {
            throw PluginException.Invalid("the import context is missing");
        }

        DateOnly? date = DateOnly.TryParseExact(ctx.FileDate, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out var d) ? d : null;
        return new ImportContext(ctx.FileName, mayReadFiles ? ctx.Path : null, date, ctx.ProfileJson);
    }

    public static JsonNode ParsePayload(string json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return new JsonObject();
        }

        try
        {
            return JsonNode.Parse(json) ?? new JsonObject();
        }
        catch (JsonException)
        {
            return new JsonObject();
        }
    }
}
