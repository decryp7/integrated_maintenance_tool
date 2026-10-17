using System.Text.RegularExpressions;

namespace Imt.Plugin;

/// <summary>A dependency on another plugin (ADR 0003).</summary>
/// <param name="Id">The other plugin's id.</param>
/// <param name="Requirement">A semver requirement, e.g. <c>^1.2</c>.</param>
/// <param name="Optional">An optional dependency only reduces features when it is missing.</param>
public sealed record Dependency(string Id, string Requirement, bool Optional);

/// <summary>A service the plugin provides, by id and semver version.</summary>
/// <param name="Id">The service id.</param>
/// <param name="Version">Its semver version.</param>
public sealed record ServiceDeclaration(string Id, string Version);

/// <summary>A plugin's identity, needs and permissions (the handshake reply without capabilities).</summary>
/// <param name="Id">Reverse-DNS id, e.g. <c>com.example.toy</c>.</param>
/// <param name="Name">The name shown to the user.</param>
/// <param name="Version">Semver version.</param>
/// <param name="Requires">Dependencies on other plugins.</param>
/// <param name="Provides">Services for other plugins.</param>
/// <param name="Permissions">What the plugin asks the user to allow.</param>
public sealed record PluginManifest(
    string Id,
    string Name,
    string Version,
    IReadOnlyList<Dependency> Requires,
    IReadOnlyList<ServiceDeclaration> Provides,
    IReadOnlyList<PermissionRequest> Permissions);

/// <summary>A checked plugin: its manifest and its contributions by capability. Built by <see cref="PluginBuilder"/>.</summary>
public sealed class PluginDefinition
{
    internal PluginDefinition(PluginManifest manifest, IReadOnlyList<object> contributions)
    {
        Manifest = manifest;
        Importers = [.. contributions.OfType<IImporter>()];
        Transforms = [.. contributions.OfType<ITransform>()];
        Exporters = [.. contributions.OfType<IExporter>()];
        MarkerKinds = [.. contributions.OfType<IMarkerKinds>().SelectMany(m => m.MarkerKinds)];
        Annotators = [.. contributions.OfType<IAnnotator>()];
        Settings = contributions.OfType<ISettingsSection>().FirstOrDefault();
        Services = [.. contributions.OfType<IPluginService>()];
        EventSubscribers = [.. contributions.OfType<IEventSubscriber>()];
        EventKinds = [.. EventSubscribers.SelectMany(s => s.EventKinds).Distinct(StringComparer.Ordinal)];
    }

    /// <summary>The manifest.</summary>
    public PluginManifest Manifest { get; }

    /// <summary>The importers.</summary>
    public IReadOnlyList<IImporter> Importers { get; }

    /// <summary>The transforms.</summary>
    public IReadOnlyList<ITransform> Transforms { get; }

    /// <summary>The exporters.</summary>
    public IReadOnlyList<IExporter> Exporters { get; }

    /// <summary>Every marker kind of every <see cref="IMarkerKinds"/> contribution.</summary>
    public IReadOnlyList<MarkerKind> MarkerKinds { get; }

    /// <summary>The annotators.</summary>
    public IReadOnlyList<IAnnotator> Annotators { get; }

    /// <summary>The settings section, if any.</summary>
    public ISettingsSection? Settings { get; }

    /// <summary>The services.</summary>
    public IReadOnlyList<IPluginService> Services { get; }

    /// <summary>The event subscribers.</summary>
    public IReadOnlyList<IEventSubscriber> EventSubscribers { get; }

    /// <summary>Every event kind any subscriber wants.</summary>
    public IReadOnlyList<string> EventKinds { get; }
}

/// <summary>
/// Builds a plugin fluently: identity, dependencies, permissions and contributions. Capabilities are derived
/// from the interfaces each contribution implements (<see cref="IImporter"/>, <see cref="ITransform"/>,
/// <see cref="IMarkerKinds"/>, <see cref="IAnnotator"/>, <see cref="IExporter"/>, <see cref="ISettingsSection"/>,
/// <see cref="IPluginService"/>, <see cref="IEventSubscriber"/>); one object may implement several.
/// <see cref="Build"/> checks everything IMT checks, so a mistake fails at start with a clear message.
/// </summary>
public sealed partial class PluginBuilder
{
    private const int MaxList = 1000;
    private readonly string id;
    private readonly List<Dependency> requires = [];
    private readonly List<ServiceDeclaration> provides = [];
    private readonly List<PermissionRequest> permissions = [];
    private readonly List<object> contributions = [];
    private string? name;
    private string version = "0.1.0";

    private PluginBuilder(string id) => this.id = id;

    /// <summary>Starts a plugin.</summary>
    /// <param name="id">Reverse-DNS id, e.g. <c>com.example.toy</c> (1–128 characters from A–Z, a–z, 0–9, '.', '_', '-', '/').</param>
    /// <returns>The builder.</returns>
    public static PluginBuilder Create(string id) => new(id);

    /// <summary>Sets the name shown to the user (default: the id).</summary>
    /// <param name="value">The name.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Name(string value)
    {
        name = value;
        return this;
    }

    /// <summary>Sets the semver version (default <c>0.1.0</c>).</summary>
    /// <param name="value">The version, e.g. <c>1.2.0</c>.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Version(string value)
    {
        version = value;
        return this;
    }

    /// <summary>Requires another plugin; without it this plugin stays inactive.</summary>
    /// <param name="pluginId">The other plugin's id.</param>
    /// <param name="requirement">A semver requirement, e.g. <c>^1.2</c>.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Requires(string pluginId, string requirement)
    {
        requires.Add(new Dependency(pluginId, requirement, false));
        return this;
    }

    /// <summary>Uses another plugin when it is there; without it only some features are missing.</summary>
    /// <param name="pluginId">The other plugin's id.</param>
    /// <param name="requirement">A semver requirement, e.g. <c>^1.2</c>.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Optionally(string pluginId, string requirement)
    {
        requires.Add(new Dependency(pluginId, requirement, true));
        return this;
    }

    /// <summary>Declares a service id and version this plugin provides (services added with <see cref="Add"/> are declared automatically).</summary>
    /// <param name="serviceId">The service id.</param>
    /// <param name="serviceVersion">Its semver version.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Provides(string serviceId, string serviceVersion)
    {
        provides.Add(new ServiceDeclaration(serviceId, serviceVersion));
        return this;
    }

    /// <summary>Asks the user for a permission (shown at install with the reason; enforced by IMT).</summary>
    /// <param name="permission">The permission.</param>
    /// <param name="reason">Why the plugin needs it.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Permission(Permission permission, string reason)
    {
        permissions.Add(new PermissionRequest(permission, reason));
        return this;
    }

    /// <summary>Adds a contribution; its capabilities come from the interfaces it implements.</summary>
    /// <param name="contribution">An importer, transform, exporter, marker kinds, annotator, settings section, service or event subscriber.</param>
    /// <returns>The builder.</returns>
    public PluginBuilder Add(object contribution)
    {
        ArgumentNullException.ThrowIfNull(contribution);
        if (contribution is not (IImporter or ITransform or IExporter or IMarkerKinds or IAnnotator or ISettingsSection or IPluginService or IEventSubscriber))
        {
            throw new ArgumentException(
                $"{contribution.GetType().Name} implements no capability interface (IImporter, ITransform, IExporter, IMarkerKinds, IAnnotator, ISettingsSection, IPluginService, IEventSubscriber)",
                nameof(contribution));
        }

        contributions.Add(contribution);
        return this;
    }

    /// <summary>Checks everything and returns the plugin.</summary>
    /// <returns>The plugin.</returns>
    /// <exception cref="InvalidOperationException">Something is not valid; the message says what to fix.</exception>
    public PluginDefinition Build()
    {
        CheckId("plugin id", id);
        if (!SemVer().IsMatch(version))
        {
            Fail($"plugin \"{id}\": version \"{version}\" is not semver (e.g. 1.2.0)");
        }

        foreach (var d in requires)
        {
            CheckId("dependency", d.Id);
            if (string.IsNullOrWhiteSpace(d.Requirement))
            {
                Fail($"dependency \"{d.Id}\": the version requirement is empty (e.g. ^1.0)");
            }
        }

        var allProvides = new List<ServiceDeclaration>(provides);
        foreach (var s in contributions.OfType<IPluginService>())
        {
            if (!allProvides.Any(p => p.Id == s.ServiceId))
            {
                allProvides.Add(new ServiceDeclaration(s.ServiceId, s.ServiceVersion));
            }
        }

        foreach (var p in allProvides)
        {
            CheckId("service", p.Id);
            if (!SemVer().IsMatch(p.Version))
            {
                Fail($"service \"{p.Id}\": version \"{p.Version}\" is not semver");
            }
        }

        foreach (var p in permissions)
        {
            if (p.Permission.Kind == PermissionKind.Network
                && (p.Permission.Target.Length > 253 || !p.Permission.Target.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-' or '*')))
            {
                Fail($"permission: \"{p.Permission.Target}\" is not a host name");
            }
        }

        if (contributions.Count == 0)
        {
            Fail($"plugin \"{id}\" contributes nothing; add an importer, transform, exporter, marker kinds, annotator, settings, service or event subscriber");
        }

        var definition = new PluginDefinition(
            new PluginManifest(id, string.IsNullOrWhiteSpace(name) ? id : name, version, [.. requires], allProvides, [.. permissions]),
            [.. contributions]);
        Check(definition);
        return definition;
    }

    private static void Check(PluginDefinition d)
    {
        Unique("importer", d.Importers.Select(i => i.Id));
        Unique("transform", d.Transforms.Select(t => t.Id));
        Unique("exporter", d.Exporters.Select(e => e.Id));
        Unique("annotator", d.Annotators.Select(a => a.Id));
        Unique("service", d.Services.Select(s => s.ServiceId));
        foreach (var e in d.Exporters)
        {
            if (e.Extension.Length is 0 or > 16 || !e.Extension.All(char.IsAsciiLetterOrDigit))
            {
                Fail($"exporter \"{e.Id}\": extension \"{e.Extension}\" is not valid (1–16 letters or digits, no dot)");
            }
        }

        var kinds = new HashSet<string>(StringComparer.Ordinal);
        foreach (var k in d.MarkerKinds)
        {
            if (k.Kind.Length is 0 or > 64 || !k.Kind.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '_' or '-'))
            {
                Fail($"marker kind \"{k.Kind}\": use 1–64 characters from A–Z, a–z, 0–9, '.', '_' and '-'");
            }

            if (!kinds.Add(k.Kind))
            {
                Fail($"marker kind \"{k.Kind}\" is declared twice");
            }

            foreach (var c in k.When)
            {
                CheckCondition($"marker kind \"{k.Kind}\"", c);
            }

            CheckStyle(k);
        }

        if (kinds.Count > 64)
        {
            Fail($"{kinds.Count} marker kinds: at most 64 fit");
        }

        foreach (var a in d.Annotators)
        {
            if (a.Limit is < 0 or > Contract.MaxViewRecords)
            {
                Fail($"annotator \"{a.Id}\": limit must be 0–{Contract.MaxViewRecords}");
            }

            foreach (var c in a.Interest)
            {
                CheckCondition($"annotator \"{a.Id}\"", c);
            }
        }

        foreach (var i in d.Importers.Where(i => i.Binary && i is ITailImporter))
        {
            Fail($"importer \"{i.Id}\": binary importers cannot live-tail; remove ITailImporter or Binary");
        }

        if (d.Settings is not null)
        {
            if (d.Settings.Section is null)
            {
                Fail("the settings section is null");
            }

            if (SettingsValidation.CheckSchema(d.Settings.Section!) is { } e)
            {
                Fail($"invalid settings schema: {e}");
            }

            if (d.Settings.Section!.Fields.Count == 0)
            {
                Fail("the settings section has no fields");
            }
        }

        if (d.EventKinds.Count > 64)
        {
            Fail("at most 64 event kinds");
        }

        var counts = new[] { d.Importers.Count, d.Transforms.Count, d.Exporters.Count, d.Annotators.Count, d.Services.Count };
        if (counts.Sum() > MaxList)
        {
            Fail($"more than {MaxList} capabilities");
        }
    }

    private static readonly string[] KnownOps =
        ["eq", "=", "==", "ne", "!=", "contains", "~", "notContains", "not_contains", "!~", "matches", "regex", "exists", "missing", "gt", ">", "lt", "<", "in"];

    private static void CheckCondition(string owner, Condition c)
    {
        if (string.IsNullOrEmpty(c.Field))
        {
            Fail($"{owner}: a condition has no field");
        }

        if (!KnownOps.Contains(c.Op, StringComparer.Ordinal))
        {
            Fail($"{owner}: unknown condition operator \"{c.Op}\" (use eq, ne, contains, notContains, regex, exists, missing, gt, lt, in)");
        }
    }

    /// <summary>Icons a marker may use (IMT's <c>MARKER_ICONS</c>).</summary>
    public static IReadOnlyList<string> MarkerIcons { get; } =
    [
        "alert", "bug", "flag", "star", "pin", "bookmark", "info", "check", "x-circle", "user", "users", "lock", "key", "shield",
        "database", "network", "cloud", "server", "credit-card", "cart", "mail", "clock", "timer", "rocket", "power", "restart",
        "cpu", "memory", "disk", "file", "download", "upload", "link", "zap", "fire", "heart",
    ];

    /// <summary>Badge shapes (IMT's <c>BADGE_SHAPES</c>).</summary>
    public static IReadOnlyList<string> BadgeShapes { get; } = ["circle", "square", "diamond", "hexagon"];

    private static void CheckStyle(MarkerKind k)
    {
        if (k.Icon is { } icon && !MarkerIcons.Contains(icon, StringComparer.Ordinal))
        {
            Fail($"marker kind \"{k.Kind}\": unknown icon \"{icon}\" (use one of: {string.Join(", ", MarkerIcons)})");
        }

        if (k.Badge is { } badge)
        {
            var b = badge.Trim();
            if (b.Length is 0 or > 3 || b.Any(char.IsWhiteSpace) || b.Any(char.IsControl))
            {
                Fail($"marker kind \"{k.Kind}\": badge \"{badge}\" must be 1–3 visible characters (e.g. \"1\", \"A\", \"DB\")");
            }
        }

        if (k.Shape is { } shape && !BadgeShapes.Contains(shape, StringComparer.Ordinal))
        {
            Fail($"marker kind \"{k.Kind}\": shape \"{shape}\" must be one of {string.Join(", ", BadgeShapes)}");
        }

        if (k.Color is { } color && !HexColor().IsMatch(color))
        {
            Fail($"marker kind \"{k.Kind}\": color \"{color}\" must be #RGB or #RRGGBB");
        }
    }

    [GeneratedRegex("^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$", RegexOptions.CultureInvariant)]
    private static partial Regex HexColor();

    private static void Unique(string what, IEnumerable<string> ids)
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var i in ids)
        {
            CheckId(what, i);
            if (!seen.Add(i))
            {
                Fail($"{what} \"{i}\" is declared twice");
            }
        }
    }

    private static void CheckId(string what, string value)
    {
        if (string.IsNullOrEmpty(value) || value.Length > 128 || !value.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '_' or '-' or '/'))
        {
            Fail($"{what} \"{value}\" is not a valid id (1–128 characters from A–Z, a–z, 0–9, '.', '_', '-', '/')");
        }
    }

    [System.Diagnostics.CodeAnalysis.DoesNotReturn]
    private static void Fail(string message) => throw new InvalidOperationException(message);

    [GeneratedRegex(@"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$", RegexOptions.CultureInvariant)]
    private static partial Regex SemVer();
}
