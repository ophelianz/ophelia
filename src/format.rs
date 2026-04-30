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

use std::fmt;

const KB: f64 = 1_000.0;
const MB: f64 = 1_000_000.0;
const GB: f64 = 1_000_000_000.0;
const TB: f64 = 1_000_000_000_000.0;

#[derive(Clone, Copy, Debug)]
pub enum DataQuantity {
    Bytes(u64),
    StorageBytes(u64),
    BytesPerSecond(u64),
    MegabytesPerSecond(f32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataLabel {
    pub value: String,
    pub unit: &'static str,
}

impl fmt::Display for DataLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.value, self.unit)
    }
}

pub fn data(quantity: DataQuantity) -> DataLabel {
    match quantity {
        DataQuantity::Bytes(bytes) => bytes_label(bytes),
        DataQuantity::StorageBytes(bytes) => storage_label(bytes),
        DataQuantity::BytesPerSecond(speed_bps) => speed_label(speed_bps),
        DataQuantity::MegabytesPerSecond(speed_mbs) => DataLabel {
            value: format!("{speed_mbs:.1}"),
            unit: "MB/s",
        },
    }
}

fn bytes_label(bytes: u64) -> DataLabel {
    let bytes = bytes as f64;
    if bytes >= TB {
        label(bytes / TB, 1, "TB")
    } else if bytes >= GB {
        label(bytes / GB, 1, "GB")
    } else if bytes >= MB {
        label(bytes / MB, 1, "MB")
    } else if bytes >= KB {
        label(bytes / KB, 0, "KB")
    } else {
        label(bytes, 0, "B")
    }
}

fn storage_label(bytes: u64) -> DataLabel {
    let bytes = bytes as f64;
    if bytes >= TB {
        label(bytes / TB, 1, "TB")
    } else {
        label(bytes / GB, 1, "GB")
    }
}

fn speed_label(speed_bps: u64) -> DataLabel {
    let speed = speed_bps as f64;
    if speed >= TB {
        label(speed / TB, 1, "TB/s")
    } else if speed >= GB {
        label(speed / GB, 1, "GB/s")
    } else if speed >= MB {
        label(speed / MB, 1, "MB/s")
    } else if speed >= KB {
        label(speed / KB, 1, "KB/s")
    } else {
        label(speed, 0, "B/s")
    }
}

fn label(value: f64, decimals: usize, unit: &'static str) -> DataLabel {
    DataLabel {
        value: format!("{value:.decimals$}"),
        unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_human_readable_byte_sizes() {
        assert_eq!(data(DataQuantity::Bytes(999)).to_string(), "999 B");
        assert_eq!(data(DataQuantity::Bytes(12_300)).to_string(), "12 KB");
        assert_eq!(data(DataQuantity::Bytes(4_500_000)).to_string(), "4.5 MB");
        assert_eq!(
            data(DataQuantity::Bytes(2_300_000_000)).to_string(),
            "2.3 GB"
        );
    }

    #[test]
    fn formats_storage_sizes_as_gb_or_tb() {
        assert_eq!(
            data(DataQuantity::StorageBytes(512_000_000_000)).to_string(),
            "512.0 GB"
        );
        assert_eq!(
            data(DataQuantity::StorageBytes(1_500_000_000_000)).to_string(),
            "1.5 TB"
        );
    }

    #[test]
    fn formats_human_readable_speeds() {
        assert_eq!(
            data(DataQuantity::BytesPerSecond(512)).to_string(),
            "512 B/s"
        );
        assert_eq!(
            data(DataQuantity::BytesPerSecond(1_500)).to_string(),
            "1.5 KB/s"
        );
        assert_eq!(
            data(DataQuantity::BytesPerSecond(12_300_000)).to_string(),
            "12.3 MB/s"
        );
        assert_eq!(
            data(DataQuantity::BytesPerSecond(2_300_000_000)).to_string(),
            "2.3 GB/s"
        );
    }

    #[test]
    fn formats_megabytes_per_second_labels() {
        let label = data(DataQuantity::MegabytesPerSecond(12.34));

        assert_eq!(label.value, "12.3");
        assert_eq!(label.unit, "MB/s");
        assert_eq!(label.to_string(), "12.3 MB/s");
    }
}
