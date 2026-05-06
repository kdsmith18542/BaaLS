use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::System;
use tokio::time::interval;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: String,
    pub uptime_seconds: u64,
    pub version: String,
    pub storage_healthy: bool,
    pub memory_usage_mb: f64,
    pub connected_peers: usize,
    pub latest_block_index: u64,
    pub latest_block_hash: String,
    pub mempool_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub block_processing_time: Vec<Duration>,
    pub transaction_validation_time: Vec<Duration>,
    pub storage_operation_time: Vec<Duration>,
    pub mempool_operations: u64,
    pub total_blocks_processed: u64,
    pub total_transactions_processed: u64,
    pub average_block_size: f64,
    pub peak_memory_usage: u64,
    pub uptime: Duration,
    // New: Enhanced metrics for Phase 4
    pub contract_execution_time: Vec<Duration>,
    pub network_sync_time: Vec<Duration>,
    pub consensus_operations: u64,
    pub storage_compaction_time: Vec<Duration>,
    pub error_count: HashMap<String, u64>,
    pub throughput_tps: f64,
    pub latency_p95: Duration,
    pub latency_p99: Duration,
    pub memory_usage_history: Vec<MemorySnapshot>,
    pub cpu_usage_history: Vec<CpuSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub timestamp: u64,
    pub used_memory: u64,
    pub available_memory: u64,
    pub heap_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuSnapshot {
    pub timestamp: u64,
    pub cpu_usage_percent: f64,
    pub load_average: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationReport {
    pub bottlenecks: Vec<Bottleneck>,
    pub recommendations: Vec<Recommendation>,
    pub performance_score: f64,
    pub resource_utilization: ResourceUtilization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub component: String,
    pub severity: BottleneckSeverity,
    pub description: String,
    pub impact_score: f64,
    pub suggested_fix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BottleneckSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub category: String,
    pub description: String,
    pub expected_improvement: f64,
    pub implementation_effort: ImplementationEffort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImplementationEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUtilization {
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
    pub network_usage: f64,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            block_processing_time: Vec::new(),
            transaction_validation_time: Vec::new(),
            storage_operation_time: Vec::new(),
            mempool_operations: 0,
            total_blocks_processed: 0,
            total_transactions_processed: 0,
            average_block_size: 0.0,
            peak_memory_usage: 0,
            uptime: Duration::ZERO,
            contract_execution_time: Vec::new(),
            network_sync_time: Vec::new(),
            consensus_operations: 0,
            storage_compaction_time: Vec::new(),
            error_count: HashMap::new(),
            throughput_tps: 0.0,
            latency_p95: Duration::ZERO,
            latency_p99: Duration::ZERO,
            memory_usage_history: Vec::new(),
            cpu_usage_history: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct MetricsCollector {
    metrics: Arc<Mutex<PerformanceMetrics>>,
    start_time: Instant,
    background_monitor: Option<thread::JoinHandle<()>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(PerformanceMetrics::default())),
            start_time: Instant::now(),
            background_monitor: None,
        }
    }

    pub fn start_background_monitoring(&mut self) {
        let metrics = self.metrics.clone();
        let handle = thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut interval = interval(Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    Self::collect_system_metrics(&metrics).await;
                }
            });
        });
        self.background_monitor = Some(handle);
    }

    async fn collect_system_metrics(metrics: &Arc<Mutex<PerformanceMetrics>>) {
        // Collect memory usage
        let mut sys = System::new();
        sys.refresh_memory();
        let mut metrics = metrics.lock().unwrap();
        let snapshot = MemorySnapshot {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            used_memory: sys.used_memory(),
            available_memory: sys.available_memory(),
            heap_size: sys.total_memory(),
        };
        metrics.memory_usage_history.push(snapshot);

        // Keep only last 1000 snapshots
        if metrics.memory_usage_history.len() > 1000 {
            metrics.memory_usage_history.remove(0);
        }
    }

    pub fn record_block_processing(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.block_processing_time.push(duration);
        metrics.total_blocks_processed += 1;

        // Keep only last 1000 measurements to prevent memory bloat
        if metrics.block_processing_time.len() > 1000 {
            metrics.block_processing_time.remove(0);
        }

        // Update throughput
        self.update_throughput(&mut metrics);
    }

    pub fn record_transaction_validation(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.transaction_validation_time.push(duration);
        metrics.total_transactions_processed += 1;

        if metrics.transaction_validation_time.len() > 1000 {
            metrics.transaction_validation_time.remove(0);
        }

        // Update latency percentiles
        self.update_latency_percentiles(&mut metrics);
    }

    pub fn record_storage_operation(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.storage_operation_time.push(duration);

        if metrics.storage_operation_time.len() > 1000 {
            metrics.storage_operation_time.remove(0);
        }
    }

    pub fn record_contract_execution(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.contract_execution_time.push(duration);

        if metrics.contract_execution_time.len() > 1000 {
            metrics.contract_execution_time.remove(0);
        }
    }

    pub fn record_network_sync(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.network_sync_time.push(duration);

        if metrics.network_sync_time.len() > 1000 {
            metrics.network_sync_time.remove(0);
        }
    }

    pub fn record_error(&self, error_type: &str) {
        let mut metrics = self.metrics.lock().unwrap();
        *metrics.error_count.entry(error_type.to_string()).or_insert(0) += 1;
    }

    pub fn record_mempool_operation(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.mempool_operations += 1;
    }

    pub fn record_consensus_operation(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.consensus_operations += 1;
    }

    pub fn update_average_block_size(&self, block_size: usize) {
        let mut metrics = self.metrics.lock().unwrap();
        let current_avg = metrics.average_block_size;
        let total_blocks = metrics.total_blocks_processed as f64;

        if total_blocks > 0.0 {
            metrics.average_block_size =
                (current_avg * (total_blocks - 1.0) + block_size as f64) / total_blocks;
        } else {
            metrics.average_block_size = block_size as f64;
        }
    }

    fn update_throughput(&self, metrics: &mut PerformanceMetrics) {
        let uptime_seconds = self.start_time.elapsed().as_secs() as f64;
        if uptime_seconds > 0.0 {
            metrics.throughput_tps = metrics.total_transactions_processed as f64 / uptime_seconds;
        }
    }

    fn update_latency_percentiles(&self, metrics: &mut PerformanceMetrics) {
        if !metrics.transaction_validation_time.is_empty() {
            let mut times: Vec<Duration> = metrics.transaction_validation_time.clone();
            times.sort();

            let p95_index = (times.len() as f64 * 0.95) as usize;
            let p99_index = (times.len() as f64 * 0.99) as usize;

            if p95_index < times.len() {
                metrics.latency_p95 = times[p95_index];
            }
            if p99_index < times.len() {
                metrics.latency_p99 = times[p99_index];
            }
        }
    }

    pub fn get_metrics(&self) -> PerformanceMetrics {
        let mut metrics = self.metrics.lock().unwrap().clone();
        metrics.uptime = self.start_time.elapsed();
        metrics
    }

    pub fn get_summary(&self) -> HashMap<String, f64> {
        let metrics = self.get_metrics();
        let mut summary = HashMap::new();

        summary.insert("total_blocks".to_string(), metrics.total_blocks_processed as f64);
        summary
            .insert("total_transactions".to_string(), metrics.total_transactions_processed as f64);
        summary.insert("throughput_tps".to_string(), metrics.throughput_tps);
        summary.insert("uptime_seconds".to_string(), metrics.uptime.as_secs() as f64);
        summary.insert("average_block_size".to_string(), metrics.average_block_size);
        summary.insert(
            "peak_memory_mb".to_string(),
            metrics.peak_memory_usage as f64 / 1024.0 / 1024.0,
        );

        // Calculate average processing times
        if !metrics.block_processing_time.is_empty() {
            let avg_block_time: Duration = metrics.block_processing_time.iter().sum::<Duration>()
                / metrics.block_processing_time.len() as u32;
            summary
                .insert("avg_block_processing_ms".to_string(), avg_block_time.as_millis() as f64);
        }

        if !metrics.transaction_validation_time.is_empty() {
            let avg_tx_time: Duration =
                metrics.transaction_validation_time.iter().sum::<Duration>()
                    / metrics.transaction_validation_time.len() as u32;
            summary.insert(
                "avg_transaction_validation_ms".to_string(),
                avg_tx_time.as_millis() as f64,
            );
        }

        summary.insert("latency_p95_ms".to_string(), metrics.latency_p95.as_millis() as f64);
        summary.insert("latency_p99_ms".to_string(), metrics.latency_p99.as_millis() as f64);

        summary
    }

    pub fn generate_optimization_report(&self) -> OptimizationReport {
        let metrics = self.get_metrics();
        let mut bottlenecks = Vec::new();
        let mut recommendations = Vec::new();

        // Analyze bottlenecks
        if metrics.latency_p95 > Duration::from_millis(100) {
            bottlenecks.push(Bottleneck {
                component: "Transaction Processing".to_string(),
                severity: BottleneckSeverity::Medium,
                description: "High transaction latency detected".to_string(),
                impact_score: 0.7,
                suggested_fix:
                    "Consider optimizing transaction validation or increasing processing capacity"
                        .to_string(),
            });
        }

        if metrics.throughput_tps < 100.0 {
            bottlenecks.push(Bottleneck {
                component: "Throughput".to_string(),
                severity: BottleneckSeverity::High,
                description: "Low transaction throughput".to_string(),
                impact_score: 0.9,
                suggested_fix: "Optimize block processing, increase block size, or improve consensus efficiency".to_string(),
            });
        }

        // Generate recommendations
        if !bottlenecks.is_empty() {
            recommendations.push(Recommendation {
                category: "Performance".to_string(),
                description: "Implement caching for frequently accessed data".to_string(),
                expected_improvement: 0.3,
                implementation_effort: ImplementationEffort::Medium,
            });
        }

        let performance_score = self.calculate_performance_score(&metrics);

        OptimizationReport {
            bottlenecks,
            recommendations,
            performance_score,
            resource_utilization: ResourceUtilization {
                cpu_usage: 0.0, // Would be calculated from system metrics
                memory_usage: 0.0,
                disk_usage: 0.0,
                network_usage: 0.0,
            },
        }
    }

    fn calculate_performance_score(&self, metrics: &PerformanceMetrics) -> f64 {
        let mut score = 1.0;

        // Penalize high latency
        if metrics.latency_p95 > Duration::from_millis(50) {
            score -= 0.2;
        }

        // Penalize low throughput
        if metrics.throughput_tps < 100.0 {
            score -= 0.3;
        }

        // Penalize high error rate
        let total_errors: u64 = metrics.error_count.values().sum();
        if total_errors > 0 {
            let error_rate = total_errors as f64 / metrics.total_transactions_processed as f64;
            score -= error_rate * 0.5;
        }

        score.max(0.0)
    }

    pub fn reset(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        *metrics = PerformanceMetrics::default();
    }

    pub fn health_check(&self) -> HealthStatus {
        let metrics = self.get_metrics();
        let mut sys = System::new();
        sys.refresh_memory();
        let used_memory = sys.used_memory() as f64 / (1024.0 * 1024.0);

        let (storage_healthy, latest_block_index, latest_block_hash, mempool_size) = {
            let m = self.metrics.lock().unwrap();
            let healthy = m.error_count.values().sum::<u64>() == 0 || m.total_blocks_processed > 0;
            (healthy, m.total_blocks_processed, String::new(), 0usize)
        };

        let status = if !storage_healthy || used_memory > 4096.0 {
            "unhealthy".to_string()
        } else if used_memory > 2048.0 {
            "degraded".to_string()
        } else {
            "healthy".to_string()
        };

        HealthStatus {
            status,
            uptime_seconds: metrics.uptime.as_secs(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            storage_healthy,
            memory_usage_mb: used_memory,
            connected_peers: 0,
            latest_block_index,
            latest_block_hash,
            mempool_size,
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

pub fn time_operation_fn<T, F>(
    collector: &MetricsCollector,
    op: F,
    metric_fn: fn(&MetricsCollector, std::time::Duration),
) -> T
where
    F: FnOnce() -> T,
{
    let start = Instant::now();
    let result = op();
    let duration = start.elapsed();
    metric_fn(collector, duration);
    result
}

// New: Performance profiling utilities for Phase 4
pub struct PerformanceProfiler {
    collector: MetricsCollector,
    profiling_enabled: bool,
}

impl Default for PerformanceProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceProfiler {
    pub fn new() -> Self {
        Self { collector: MetricsCollector::new(), profiling_enabled: true }
    }

    pub fn enable_profiling(&mut self) {
        self.profiling_enabled = true;
    }

    pub fn disable_profiling(&mut self) {
        self.profiling_enabled = false;
    }

    pub fn profile_operation<T, F>(&self, operation_name: &str, operation: F) -> T
    where
        F: FnOnce() -> T,
    {
        if !self.profiling_enabled {
            return operation();
        }

        let start = Instant::now();
        let result = operation();
        let duration = start.elapsed();

        // Record the operation based on its type
        match operation_name {
            "block_processing" => self.collector.record_block_processing(duration),
            "transaction_validation" => self.collector.record_transaction_validation(duration),
            "storage_operation" => self.collector.record_storage_operation(duration),
            "contract_execution" => self.collector.record_contract_execution(duration),
            "network_sync" => self.collector.record_network_sync(duration),
            _ => {} // Unknown operation type
        }

        result
    }

    pub fn get_collector(&self) -> &MetricsCollector {
        &self.collector
    }
}
