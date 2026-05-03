use serde::{Deserialize, Serialize};
use std::{num::NonZeroUsize, time::Duration};

pub(crate) const DEFAULT_SAMPLES: usize = 10;
pub(crate) const DEFAULT_MIN_SAMPLE_MS: u64 = 250;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Importance {
    Critical = 4,
    Important = 3,
    #[default]
    Average = 2,
    Iffy = 1,
    Fluff = 0,
}

impl Importance {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "critical" => Self::Critical,
            "important" => Self::Important,
            "average" => Self::Average,
            "iffy" => Self::Iffy,
            "fluff" => Self::Fluff,
            _ => return None,
        })
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Important => "important",
            Self::Average => "average",
            Self::Iffy => "iffy",
            Self::Fluff => "fluff",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TestMetadata {
    pub(crate) iterations: Option<NonZeroUsize>,
    pub(crate) importance: Importance,
    pub(crate) weight: u8,
}

impl Default for TestMetadata {
    fn default() -> Self {
        Self {
            iterations: None,
            importance: Importance::Average,
            weight: 50,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TimingResult {
    pub(crate) mean_nanos: u128,
    pub(crate) stddev_nanos: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum FailureKind {
    BadMetadata,
    Skipped,
    Triage,
    Run,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TestResult {
    pub(crate) name: String,
    pub(crate) metadata: Option<TestMetadata>,
    pub(crate) iterations: Option<NonZeroUsize>,
    pub(crate) result: Result<TimingResult, FailureKind>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PerfRun {
    pub(crate) tests: Vec<TestResult>,
}

impl PerfRun {
    pub(crate) fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    pub(crate) fn push_success(
        &mut self,
        name: String,
        metadata: TestMetadata,
        iterations: NonZeroUsize,
        result: TimingResult,
    ) {
        self.tests.push(TestResult {
            name,
            metadata: Some(metadata),
            iterations: Some(iterations),
            result: Ok(result),
        });
    }

    pub(crate) fn push_failure(
        &mut self,
        name: String,
        metadata: Option<TestMetadata>,
        iterations: Option<NonZeroUsize>,
        kind: FailureKind,
    ) {
        self.tests.push(TestResult {
            name,
            metadata,
            iterations,
            result: Err(kind),
        });
    }

    pub(crate) fn merge_prefixed(&mut self, other: Self, prefix: &str) {
        self.tests.extend(other.tests.into_iter().map(|mut test| {
            test.name = format!("{prefix}::{}", test.name);
            test
        }));
    }
}

pub(crate) struct Options {
    pub(crate) min_importance: Importance,
    pub(crate) output_json: Option<String>,
    pub(crate) quiet: bool,
    pub(crate) samples: usize,
    pub(crate) min_sample_time: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_importance: Importance::Iffy,
            output_json: None,
            quiet: false,
            samples: DEFAULT_SAMPLES,
            min_sample_time: Duration::from_millis(DEFAULT_MIN_SAMPLE_MS),
        }
    }
}
