//! Ready-made profiles for common log formats (`templates/*.toml`, built in),
//! each with a small sample in its format for a preview before the user's own
//! file is chosen. A template is applied like any profile: validated, then saved.

use serde::Serialize;

/// One built-in profile template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTemplate {
    pub id: &'static str,
    pub name: &'static str,
    /// Which logs it reads and what it sets up.
    pub description: &'static str,
    /// Gallery group: ".NET", "Web servers", "System" or "Generic".
    pub category: &'static str,
    /// The profile (merged over the defaults like any profile).
    pub toml: &'static str,
    /// File name of the sample, passed as the preview's file name.
    pub sample_name: &'static str,
    /// A few lines in the format; empty for a binary format (EVTX), which needs a real file.
    pub sample_text: &'static str,
}

macro_rules! template {
    ($id:literal, $name:literal, $category:literal, $description:literal, $sample:literal) => {
        ProfileTemplate {
            id: $id,
            name: $name,
            category: $category,
            description: $description,
            toml: include_str!(concat!("../templates/", $id, ".toml")),
            sample_name: $sample,
            sample_text: include_str!(concat!("../templates/samples/", $sample)),
        }
    };
}

/// Every built-in template, in gallery order.
pub const TEMPLATES: &[ProfileTemplate] = &[
    template!(
        "serilog-clef",
        "Serilog CLEF",
        ".NET",
        "Compact JSON from Serilog.Formatting.Compact (.clef): one event per line with @t, @l, @mt and @x; every property becomes a field.",
        "orders.clef"
    ),
    template!(
        "serilog-json",
        "Serilog JSON",
        ".NET",
        "JSON from Serilog's JsonFormatter: Timestamp, Level, MessageTemplate or RenderedMessage, Exception, and the Properties object as fields.",
        "gateway.json"
    ),
    template!(
        "serilog-text",
        "Serilog text",
        ".NET",
        "Text from the Serilog console or file sink with the default output template ({Timestamp} [{Level:u3}] {Message}), properties as JSON and exceptions on the next lines.",
        "worker.txt"
    ),
    template!(
        "log4net-xml",
        "log4net XML",
        ".NET",
        "XML events from log4net's XmlLayout (or log4j's schema): logger, thread, level and message, with exceptions; one event per line.",
        "billing.xml"
    ),
    template!(
        "log4net-pattern",
        "log4net pattern layout",
        ".NET",
        "Text from log4net's usual PatternLayout \"%date [%thread] %-5level %logger - %message\" (with or without %ndc); stack traces stay with their event.",
        "billing.log"
    ),
    template!(
        "nlog",
        "NLog default layout",
        ".NET",
        "Text from NLog's default layout \"${longdate}|${level}|${logger}|${message}\": logger as a field, multi-line messages kept together.",
        "nlog.log"
    ),
    template!(
        "mel-console",
        "Microsoft.Extensions.Logging console",
        ".NET",
        "ASP.NET Core / Microsoft.Extensions.Logging simple console output with a timestamp: \"info: Category[EventId]\" with the message on the indented lines below.",
        "console.log"
    ),
    template!(
        "iis-w3c",
        "IIS W3C extended",
        "Web servers",
        "IIS access logs (u_ex*.log) with the default W3C fields: method, URL, status, client and time taken; 4xx are warnings, 5xx errors.",
        "u_ex261014.log"
    ),
    template!(
        "access-combined",
        "Apache / Nginx access (combined)",
        "Web servers",
        "Access logs in the combined or common format used by Apache httpd and Nginx: client, user, request, status and size; 4xx are warnings, 5xx errors.",
        "access.log"
    ),
    template!(
        "apache-error",
        "Apache error log",
        "Web servers",
        "Apache httpd ErrorLog in the 2.2 and 2.4 layouts: the level in brackets (with the module), process, thread and client as fields.",
        "apache-error.log"
    ),
    template!(
        "syslog",
        "Syslog",
        "System",
        "Syslog lines in RFC 5424 or RFC 3164 (BSD) form: host, app and process as fields; the severity in <pri> sets the level.",
        "syslog.log"
    ),
    // EVTX is binary: there is no text sample; the preview needs one of the user's files.
    ProfileTemplate {
        id: "windows-event",
        name: "Windows Event Log (EVTX)",
        category: "System",
        description: "Windows event logs (.evtx) read by the Windows plugin: provider, event id, channel, computer and event data as fields; instances by computer and provider.",
        toml: include_str!("../templates/windows-event.toml"),
        sample_name: "System.evtx",
        sample_text: "",
    },
    template!(
        "android-logcat",
        "Android logcat",
        "System",
        "Logcat in the default threadtime format (adb logcat, bug reports): process and thread ids, tag and the one-letter level; the year is assumed.",
        "logcat.txt"
    ),
    template!(
        "hadoop-spark",
        "Hadoop / Spark (log4j)",
        "Java",
        "Spark and Hadoop with their default log4j layouts (\"yy/MM/dd HH:mm:ss LEVEL Logger: message\", and HDFS's \"yyMMdd HHmmss\"); logger as a field, stack traces kept with their event.",
        "spark.log"
    ),
    template!(
        "csv",
        "CSV with a header row",
        "Generic",
        "Comma, semicolon, tab or pipe separated files with a header: the timestamp, level and message columns are found by name; every other column becomes a field.",
        "events.csv"
    ),
    template!(
        "plain-timestamped",
        "Plain timestamped text",
        "Generic",
        "Any text log whose lines start with a date and time, optionally followed by a level; lines without a time (stack traces) continue the previous event.",
        "app.log"
    ),
];

/// The template with this id.
pub fn find(id: &str) -> Option<&'static ProfileTemplate> {
    TEMPLATES.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_samples_small() {
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TEMPLATES.len());
        for t in TEMPLATES {
            assert!(t.sample_text.len() <= 4096, "{}", t.id);
            assert!(!t.description.is_empty() && !t.name.is_empty());
        }
        assert_eq!(find("nlog").map(|t| t.name), Some("NLog default layout"));
        assert!(find("nope").is_none());
        let json = serde_json::to_value(TEMPLATES[0]).unwrap();
        assert!(json["sampleName"].is_string() && json["sampleText"].is_string() && json["toml"].is_string());
    }
}
