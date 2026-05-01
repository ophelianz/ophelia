/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use std::path::Path;

use super::DestinationRule;

struct DestinationRulePreset {
    id: &'static str,
    label: &'static str,
    folder_name: &'static str,
    icon_name: &'static str,
    extensions: &'static [&'static str],
}

const DESTINATION_RULE_PRESETS: &[DestinationRulePreset] = &[
    DestinationRulePreset {
        id: "archive",
        label: "Archives",
        folder_name: "Archives",
        icon_name: "archive",
        extensions: &[".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".tgz"],
    },
    DestinationRulePreset {
        id: "audio",
        label: "Music",
        folder_name: "Music",
        icon_name: "audio",
        extensions: &[".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a", ".opus"],
    },
    DestinationRulePreset {
        id: "book",
        label: "Books",
        folder_name: "Books",
        icon_name: "book",
        extensions: &[".epub", ".mobi", ".azw3", ".fb2"],
    },
    DestinationRulePreset {
        id: "code",
        label: "Code",
        folder_name: "Code",
        icon_name: "code",
        extensions: &[
            ".rs", ".js", ".ts", ".tsx", ".jsx", ".py", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
            ".json", ".yaml", ".yml", ".toml", ".sh", ".css",
        ],
    },
    DestinationRulePreset {
        id: "document",
        label: "Documents",
        folder_name: "Documents",
        icon_name: "document",
        extensions: &[".pdf", ".doc", ".docx", ".txt", ".rtf", ".md"],
    },
    DestinationRulePreset {
        id: "executable",
        label: "Executables",
        folder_name: "Executables",
        icon_name: "executable",
        extensions: &[
            ".exe",
            ".msi",
            ".dmg",
            ".pkg",
            ".appimage",
            ".deb",
            ".rpm",
            ".apk",
        ],
    },
    DestinationRulePreset {
        id: "font",
        label: "Fonts",
        folder_name: "Fonts",
        icon_name: "font",
        extensions: &[".ttf", ".otf", ".woff", ".woff2"],
    },
    DestinationRulePreset {
        id: "image",
        label: "Images",
        folder_name: "Images",
        icon_name: "image",
        extensions: &[
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".heic", ".avif", ".bmp", ".tiff",
        ],
    },
    DestinationRulePreset {
        id: "key",
        label: "Keys",
        folder_name: "Keys",
        icon_name: "key",
        extensions: &[".pem", ".pub", ".p12", ".pfx", ".crt", ".cer", ".asc"],
    },
    DestinationRulePreset {
        id: "mail",
        label: "Mail",
        folder_name: "Mail",
        icon_name: "mail",
        extensions: &[".eml", ".mbox", ".msg"],
    },
    DestinationRulePreset {
        id: "presentation",
        label: "Presentations",
        folder_name: "Presentations",
        icon_name: "presentation",
        extensions: &[".ppt", ".pptx", ".odp"],
    },
    DestinationRulePreset {
        id: "spreadsheet",
        label: "Spreadsheets",
        folder_name: "Spreadsheets",
        icon_name: "spreadsheet",
        extensions: &[".csv", ".tsv", ".xls", ".xlsx", ".ods"],
    },
    DestinationRulePreset {
        id: "vector",
        label: "Vectors",
        folder_name: "Vectors",
        icon_name: "vector",
        extensions: &[".svg", ".ai", ".eps"],
    },
    DestinationRulePreset {
        id: "video",
        label: "Videos",
        folder_name: "Videos",
        icon_name: "video",
        extensions: &[".mp4", ".mkv", ".mov", ".avi", ".webm", ".m4v", ".wmv"],
    },
    DestinationRulePreset {
        id: "web",
        label: "Web",
        folder_name: "Web",
        icon_name: "web",
        extensions: &[".html", ".htm", ".mhtml", ".webloc", ".url"],
    },
];

const DEFAULT_DESTINATION_RULE_PRESET_IDS: &[&str] = &[
    "archive",
    "audio",
    "document",
    "executable",
    "image",
    "video",
];

pub fn default_destination_rules(base_dir: &Path) -> Vec<DestinationRule> {
    DESTINATION_RULE_PRESETS
        .iter()
        .filter(|preset| DEFAULT_DESTINATION_RULE_PRESET_IDS.contains(&preset.id))
        .map(|preset| DestinationRule {
            id: preset.id.to_string(),
            label: preset.label.to_string(),
            enabled: true,
            target_dir: base_dir.join(preset.folder_name),
            extensions: preset
                .extensions
                .iter()
                .map(|ext| ext.to_string())
                .collect(),
            icon_name: Some(preset.icon_name.to_string()),
        })
        .collect()
}

pub fn suggested_destination_rule_icon_name(label: &str, extensions: &[String]) -> &'static str {
    for extension in extensions
        .iter()
        .filter_map(|ext| normalize_rule_extension(ext))
    {
        if let Some(preset) = DESTINATION_RULE_PRESETS.iter().find(|preset| {
            preset
                .extensions
                .iter()
                .filter_map(|candidate| normalize_rule_extension(candidate))
                .any(|candidate| candidate == extension)
        }) {
            return preset.icon_name;
        }
    }

    let normalized_label = label.trim().to_ascii_lowercase();
    if normalized_label.is_empty() {
        return "default";
    }

    DESTINATION_RULE_PRESETS
        .iter()
        .find(|preset| {
            normalized_label.contains(&preset.id.to_ascii_lowercase())
                || normalized_label.contains(&preset.label.to_ascii_lowercase())
                || normalized_label
                    .contains(&preset.label.trim_end_matches('s').to_ascii_lowercase())
        })
        .map(|preset| preset.icon_name)
        .unwrap_or("default")
}

fn normalize_rule_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with('.') {
        Some(trimmed.to_ascii_lowercase())
    } else {
        Some(format!(".{}", trimmed.to_ascii_lowercase()))
    }
}
