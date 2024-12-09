use std::fmt;

use clap::ValueEnum;
use serde::Deserialize;
use serde::Serialize;

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LatencyDistribution {
    Uniform,
    Normal,
    Pareto,
    ParetoNormal,
}

impl Default for LatencyDistribution {
    fn default() -> Self {
        Self::Uniform // Default latency distribution
    }
}

impl LatencyDistribution {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "uniform" => Some(LatencyDistribution::Uniform),
            "normal" => Some(LatencyDistribution::Normal),
            "pareto" => Some(LatencyDistribution::Pareto),
            "pareto-normal" => Some(LatencyDistribution::ParetoNormal),
            _ => None,
        }
    }
}

impl fmt::Display for LatencyDistribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LatencyDistribution::Uniform => write!(f, "uniform"),
            LatencyDistribution::Normal => write!(f, "normal"),
            LatencyDistribution::Pareto => write!(f, "pareto"),
            LatencyDistribution::ParetoNormal => write!(f, "pareto-normal"),
        }
    }
}

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PacketLossType {
    Bernoulli,
    GilbertElliott,
}

impl Default for PacketLossType {
    fn default() -> Self {
        Self::Bernoulli // Default packet loss type
    }
}

impl PacketLossType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bernouilli" => Some(PacketLossType::Bernoulli),
            "gilbert-elliott" => Some(PacketLossType::GilbertElliott),
            _ => None,
        }
    }
}

impl fmt::Display for PacketLossType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketLossType::Bernoulli => write!(f, "bernoulli"),
            PacketLossType::GilbertElliott => write!(f, "gilbert-elliott"),
        }
    }
}

#[derive(
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    ValueEnum,
    Debug,
    Serialize,
    Deserialize,
)]
pub enum BandwidthUnit {
    Bps,
    Kbps,
    Mbps,
    Gbps,
}

impl Default for BandwidthUnit {
    fn default() -> Self {
        Self::Bps // Default rate limit type
    }
}

/// Fault configuration for a scenario entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fault_name")]
pub enum FaultConfiguration {
    Latency {
        latency_distribution: String,
        latency_mean: f64,
        latency_stddev: f64,
        latency_min: f64,
        latency_max: f64,
        latency_shape: f64,
        latency_scale: f64,
    },
    PacketLoss {
        packet_loss_type: String,
        packet_loss_rate: f64,
    },
    Bandwidth {
        bandwidth_rate: u32,
    },
    Jitter {
        jitter_amplitude: f64,
        jitter_frequency: f64,
    },
    Dns {
        dns_rate: u8,
    },
}

pub struct ConnectRequest {
    pub target_host: String,
    pub target_port: u16,
}
