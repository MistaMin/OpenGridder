//! Port of `Common/Source/Metrics.hpp` and `Common/Source/Metrics.cpp`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct Meter {
    counter: AtomicU64,
    rate_1min: Mutex<f64>,
    alpha_1min: f64,
    has_ext_rates: Mutex<bool>,
    ext_rate_1min: Mutex<HashMap<String, f64>>,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    pub fn new() -> Self {
        let alpha_1min = 1.0 - (0.005f64.ln() / 60.0).exp();
        Self {
            counter: AtomicU64::new(0),
            rate_1min: Mutex::new(0.0),
            alpha_1min,
            has_ext_rates: Mutex::new(false),
            ext_rate_1min: Mutex::new(HashMap::new()),
        }
    }

    pub fn increment(&self, count: u64) {
        self.counter.fetch_add(count, Ordering::Relaxed);
    }

    pub fn rate_1min(&self) -> f64 {
        let base = *self.rate_1min.lock().unwrap();
        base + self.get_ext_rate_1min()
    }

    pub fn enable_ext_data(&self, enabled: bool) {
        *self.has_ext_rates.lock().unwrap() = enabled;
    }

    pub fn get_ext_rate_1min(&self) -> f64 {
        if !*self.has_ext_rates.lock().unwrap() {
            return 0.0;
        }
        self.ext_rate_1min.lock().unwrap().values().sum()
    }

    pub fn update_ext_rate_1min(&self, key: impl Into<String>, val: f64) {
        self.ext_rate_1min.lock().unwrap().insert(key.into(), val);
    }

    pub fn remove_ext_rate_1min(&self, key: &str) {
        self.ext_rate_1min.lock().unwrap().remove(key);
    }

    pub fn aggregate_1s(&self) {
        let c = self.counter.swap(0, Ordering::Relaxed) as f64;
        let mut r = self.rate_1min.lock().unwrap();
        *r = *r * (1.0 - self.alpha_1min) + c * self.alpha_1min;
    }
}

#[derive(Debug)]
pub struct SizeMeter {
    data: Mutex<Vec<usize>>,
    idx: Mutex<usize>,
    ninty_fifth_idx: usize,
}

impl SizeMeter {
    pub fn new(updates_per_second: usize) -> Self {
        let len = updates_per_second.max(1);
        Self {
            data: Mutex::new(vec![0; len]),
            idx: Mutex::new(0),
            ninty_fifth_idx: ((len as f64 * 0.95) as usize).min(len - 1),
        }
    }

    pub fn update(&self, size: usize) {
        let mut data = self.data.lock().unwrap();
        let mut idx = self.idx.lock().unwrap();
        let len = data.len();
        data[*idx] = size;
        *idx = (*idx + 1) % len;
    }

    pub fn aggregate_values(&self) -> (usize, usize, usize, usize) {
        let mut data = self.data.lock().unwrap().clone();
        data.sort_unstable();
        let total: usize = data.iter().sum();
        let min = *data.first().unwrap_or(&0);
        let max = *data.last().unwrap_or(&0);
        let avg = total / data.len().max(1);
        let nf = data[self.ninty_fifth_idx];
        (avg, min, max, nf)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Histogram {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub sum: f64,
    pub ninty_fifth: f64,
    pub count: usize,
    pub dist: Vec<(f64, usize)>,
}

impl Histogram {
    pub fn new(num_of_bins: usize, bin_size: f64) -> Self {
        let mut dist = Vec::with_capacity(num_of_bins + 1);
        let mut lower = 0.0;
        for _ in 0..=num_of_bins {
            dist.push((lower, 0));
            lower += bin_size;
        }
        Self {
            dist,
            ..Default::default()
        }
    }

    pub fn update_bin(&mut self, bin: usize, count: usize) {
        if let Some(entry) = self.dist.get_mut(bin) {
            entry.1 += count;
        }
    }

    pub fn to_json(&self) -> Value {
        let jdist: Vec<Value> = self
            .dist
            .iter()
            .map(|(lower, count)| json!({ "lower": lower, "count": count }))
            .collect();
        json!({
            "min": self.min,
            "max": self.max,
            "avg": self.avg,
            "sum": self.sum,
            "count": self.count,
            "95th": self.ninty_fifth,
            "dist": jdist,
        })
    }
}

#[derive(Debug)]
pub struct TimeStatistic {
    times: Mutex<Vec<f64>>,
    most_recent_times: Mutex<Vec<f64>>,
    most_recent_idx: Mutex<usize>,
    values_1min: Mutex<Vec<Histogram>>,
    num_of_bins: usize,
    bin_size: f64,
    meter: Meter,
}

impl Default for TimeStatistic {
    fn default() -> Self {
        Self::new(10, 2.0)
    }
}

impl TimeStatistic {
    pub fn new(num_of_bins: usize, bin_size: f64) -> Self {
        Self {
            times: Mutex::new(Vec::new()),
            most_recent_times: Mutex::new(vec![0.0; 32]),
            most_recent_idx: Mutex::new(0),
            values_1min: Mutex::new(Vec::new()),
            num_of_bins,
            bin_size,
            meter: Meter::new(),
        }
    }

    pub fn update(&self, ms: f64) {
        self.times.lock().unwrap().push(ms);
        let mut recent = self.most_recent_times.lock().unwrap();
        let mut idx = self.most_recent_idx.lock().unwrap();
        let len = recent.len();
        recent[*idx] = ms;
        *idx = (*idx + 1) % len;
        self.meter.increment(1);
    }

    pub fn get_most_recent_average(&self) -> f64 {
        let recent = self.most_recent_times.lock().unwrap();
        recent.iter().sum::<f64>() / (recent.len().max(1) as f64)
    }

    pub fn aggregate(&self) {
        let mut samples = {
            let mut lock = self.times.lock().unwrap();
            std::mem::take(&mut *lock)
        };
        let mut hist = Histogram::new(self.num_of_bins, self.bin_size);
        if !samples.is_empty() {
            samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            hist.count = samples.len();
            hist.min = *samples.first().unwrap_or(&0.0);
            hist.max = *samples.last().unwrap_or(&0.0);
            hist.sum = samples.iter().sum();
            hist.avg = hist.sum / (hist.count as f64);
            hist.ninty_fifth = samples[(hist.count * 95 / 100).min(hist.count - 1)];
            for &v in &samples {
                let bin = ((v / self.bin_size) as usize).min(self.num_of_bins);
                hist.update_bin(bin, 1);
            }
        }
        let mut v1m = self.values_1min.lock().unwrap();
        v1m.push(hist);
        if v1m.len() > 6 {
            v1m.remove(0);
        }
    }

    pub fn get_1min_histogram(&self) -> Histogram {
        let v1m = self.values_1min.lock().unwrap();
        v1m.last().cloned().unwrap_or_else(|| Histogram::new(self.num_of_bins, self.bin_size))
    }

    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    pub fn get_duration_guard(name: &str) -> DurationTracker {
        let stat = MetricsRegistry::global().get_time_statistic(name);
        DurationTracker::new(Some(stat))
    }
}

impl Drop for DurationTracker {
    fn drop(&mut self) {
        if !self.finished {
            self.finish();
        }
    }
}

pub struct DurationTracker {
    stat: Option<Arc<TimeStatistic>>,
    start: Instant,
    finished: bool,
}

impl DurationTracker {
    pub fn new(stat: Option<Arc<TimeStatistic>>) -> Self {
        Self {
            stat,
            start: Instant::now(),
            finished: false,
        }
    }

    pub fn update(&mut self) -> f64 {
        if self.finished {
            return 0.0;
        }
        let now = Instant::now();
        let ms = now.duration_since(self.start).as_secs_f64() * 1000.0;
        if let Some(s) = &self.stat {
            s.update(ms);
        }
        self.start = now;
        ms
    }

    pub fn reset(&mut self) {
        self.start = Instant::now();
        self.finished = false;
    }

    pub fn clear(&mut self) {
        self.finished = true;
    }

    pub fn finish(&mut self) {
        self.update();
        self.finished = true;
    }

    pub fn milliseconds_passed(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

pub struct MetricsRegistry {
    meters: Mutex<HashMap<String, Arc<Meter>>>,
    time_stats: Mutex<HashMap<String, Arc<TimeStatistic>>>,
}

impl MetricsRegistry {
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<MetricsRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            meters: Mutex::new(HashMap::new()),
            time_stats: Mutex::new(HashMap::new()),
        })
    }

    pub fn get_meter(&self, name: &str) -> Arc<Meter> {
        let mut map = self.meters.lock().unwrap();
        map.entry(name.to_string())
            .or_insert_with(|| Arc::new(Meter::new()))
            .clone()
    }

    pub fn get_time_statistic(&self, name: &str) -> Arc<TimeStatistic> {
        let mut map = self.time_stats.lock().unwrap();
        map.entry(name.to_string())
            .or_insert_with(|| Arc::new(TimeStatistic::default()))
            .clone()
    }
}
