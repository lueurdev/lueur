use std::fmt;

use clap::ValueEnum;
use serde::Deserialize;
use serde::Serialize;

use crate::config;
use crate::config::FaultConfig;
use crate::errors::ScenarioError;

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

#[derive(
    clap::ValueEnum, Clone, Debug, Serialize, Deserialize, Eq, PartialEq,
)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Ingress,
    Egress,
    #[serde(untagged)]
    Both,
}

impl Default for Direction {
    fn default() -> Self {
        Self::Ingress
    }
}

impl Direction {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ingress" => Some(Direction::Ingress),
            "egress" => Some(Direction::Egress),
            "both" => Some(Direction::Both),
            _ => None,
        }
    }

    pub fn is_ingress(&self) -> bool {
        self == &Direction::Ingress || self == &Direction::Both
    }

    pub fn is_egress(&self) -> bool {
        self == &Direction::Egress || self == &Direction::Both
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::Ingress => write!(f, "ingress"),
            Direction::Egress => write!(f, "egress"),
            Direction::Both => write!(f, "both"),
        }
    }
}


#[derive(
    clap::ValueEnum, Clone, Debug, Serialize, Deserialize, Eq, PartialEq,
)]
#[serde(rename_all = "lowercase")]
pub enum StreamSide {
    Client,
    Server
}

impl Default for StreamSide {
    fn default() -> Self {
        Self::Server
    }
}

impl fmt::Display for StreamSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamSide::Client => write!(f, "client"),
            StreamSide::Server => write!(f, "server"),
        }
    }
}

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PacketLossType {
    MultiStateMarkov,
}

impl Default for PacketLossType {
    fn default() -> Self {
        Self::MultiStateMarkov // Default packet loss type
    }
}

impl PacketLossType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "multistatemarkov" => Some(PacketLossType::MultiStateMarkov),
            _ => None,
        }
    }
}

impl fmt::Display for PacketLossType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketLossType::MultiStateMarkov => write!(f, "multistatemarkov"),
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
    Bps, // Bytes per second
    #[clap(name = "kbps")]
    KBps, // Kilobytes per second
    #[clap(name = "mbps")]
    MBps, // Megabytes per second
    #[clap(name = "gbps")]
    GBps, // Gigabytes per second
}

impl Default for BandwidthUnit {
    fn default() -> Self {
        Self::Bps // Default rate limit type
    }
}

impl fmt::Display for BandwidthUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BandwidthUnit::Bps => write!(f, "Bps"),
            BandwidthUnit::KBps => write!(f, "KBps"),
            BandwidthUnit::MBps => write!(f, "MBps"),
            BandwidthUnit::GBps => write!(f, "GBps"),
        }
    }
}

impl BandwidthUnit {
    pub fn to_bytes_per_second(&self, rate: usize) -> usize {
        match self {
            BandwidthUnit::Bps => rate,
            BandwidthUnit::KBps => rate * 1_000,
            BandwidthUnit::MBps => rate * 1_000_000,
            BandwidthUnit::GBps => rate * 1_000_000_000,
        }
    }
}

/// Fault configuration for a scenario entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FaultConfiguration {
    Latency {
        distribution: Option<String>,
        global: Option<bool>,
        side: Option<StreamSide>,
        mean: Option<f64>,
        stddev: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
        shape: Option<f64>,
        scale: Option<f64>,
        direction: Option<String>,
    },
    PacketLoss {
        packet_loss_type: String,
        packet_loss_rate: f64,
        direction: String,
        side: Option<StreamSide>
    },
    Bandwidth {
        rate: u32,
        unit: BandwidthUnit,
        direction: String,
        side: Option<StreamSide>
    },
    Jitter {
        jitter_amplitude: f64,
        jitter_frequency: f64,
        direction: String,
    },
    Dns {
        dns_rate: u8,
        direction: String,
    },
}

impl FaultConfiguration {
    pub fn build(&self) -> Result<FaultConfig, ScenarioError> {
        match self {
            FaultConfiguration::Bandwidth { rate, unit, direction, side } => {
                let settings = config::BandwidthSettings {
                    direction: Direction::from_str(direction).unwrap(),
                    side: side.clone().unwrap_or_default(),
                    bandwidth_rate: *rate as usize,
                    bandwidth_unit: unit.clone(),
                };

                Ok(FaultConfig::Bandwidth(settings))
            }
            FaultConfiguration::Latency {
                distribution,
                global,
                side,
                mean,
                stddev,
                min,
                max,
                scale,
                shape,
                direction,
            } => {
                let settings = config::LatencySettings {
                    distribution: LatencyDistribution::from_str(
                        &distribution.clone().unwrap_or("normal".to_string()),
                    )
                    .unwrap(),
                    global: global.unwrap_or(true),
                    side: side.clone().unwrap_or_default(),
                    latency_mean: mean.unwrap_or(100.0),
                    latency_stddev: stddev.unwrap_or(20.0),
                    latency_min: min.unwrap_or(20.0),
                    latency_max: max.unwrap_or(20.0),
                    latency_shape: shape.unwrap_or(20.0),
                    latency_scale: scale.unwrap_or(20.0),
                    direction: Direction::from_str(
                        &direction.clone().unwrap_or("egress".to_string()),
                    )
                    .unwrap(),
                };

                Ok(FaultConfig::Latency(settings))
            }
            FaultConfiguration::PacketLoss {
                packet_loss_type,
                packet_loss_rate,
                direction,
                side
            } => {
                let settings = config::PacketLossSettings {
                    direction: Direction::from_str(direction).unwrap(),
                    side: side.clone().unwrap_or_default(),
                };

                Ok(FaultConfig::PacketLoss(settings))
            }
            FaultConfiguration::Jitter {
                jitter_amplitude,
                jitter_frequency,
                direction,
            } => {
                let settings = config::JitterSettings {
                    direction: Direction::from_str(direction).unwrap(),
                    jitter_amplitude: *jitter_amplitude,
                    jitter_frequency: *jitter_frequency,
                };

                Ok(FaultConfig::Jitter(settings))
            }
            FaultConfiguration::Dns { dns_rate, direction } => {
                let settings = config::DnsSettings {
                    direction: Direction::from_str(direction).unwrap(),
                    dns_rate: *dns_rate,
                };

                Ok(FaultConfig::Dns(settings))
            }
        }
    }

    pub fn fault_type(&self) -> String {
        match self {
            FaultConfiguration::Latency {
                distribution: _,
                global: _,
                side: _,
                mean: _,
                stddev: _,
                min: _,
                max: _,
                shape: _,
                scale: _,
                direction: _,
            } => "latency".to_string(),
            FaultConfiguration::PacketLoss {
                packet_loss_type: _,
                packet_loss_rate: _,
                direction: _,
                side: _,
            } => "packetloss".to_string(),
            FaultConfiguration::Bandwidth {
                rate: _,
                unit: _,
                direction: _,
                side: _,
            } => "bandwidth".to_string(),
            FaultConfiguration::Jitter {
                jitter_amplitude: _,
                jitter_frequency: _,
                direction: _,
            } => "jitter".to_string(),
            FaultConfiguration::Dns { dns_rate: _, direction: _ } => {
                "dns".to_string()
            }
        }
    }
}

pub struct ConnectRequest {
    pub target_host: String,
    pub target_port: u16,
}

impl fmt::Display for FaultConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaultConfiguration::Latency {
                distribution,
                global,
                side,
                mean,
                stddev,
                min,
                max,
                shape,
                scale,
                direction,
            } => {
                write!(f, "Latency Fault")?;
                let mut details = Vec::new();

                details.push(format!("Global: {}", global.unwrap_or(true)));
                details.push(format!("Side: {}", side.clone().unwrap_or_default()));

                if let Some(dist) = distribution {
                    details.push(format!("Distribution: {}", dist));
                }
                if let Some(m) = mean {
                    details.push(format!("Mean: {:.2} ms", m));
                }
                if let Some(s) = stddev {
                    details.push(format!("Stddev: {:.2} ms", s));
                }
                if let Some(min) = min {
                    details.push(format!("Min: {:.2} ms", min));
                }
                if let Some(max) = max {
                    details.push(format!("Max: {:.2} ms", max));
                }
                if let Some(shape) = shape {
                    details.push(format!("Shape: {:.2}", shape));
                }
                if let Some(scale) = scale {
                    details.push(format!("Scale: {:.2}", scale));
                }
                if let Some(dir) = direction {
                    details.push(format!("Direction: {}", dir));
                }

                if !details.is_empty() {
                    write!(f, " [")?;
                    write!(f, "{}", details.join(", "))?;
                    write!(f, "]")?;
                }

                Ok(())
            }
            FaultConfiguration::PacketLoss {
                packet_loss_type,
                packet_loss_rate,
                direction,
                side: _,
            } => {
                write!(
                    f,
                    "Packet Loss Fault - Type: {}, Rate: {:.2}%, Direction: {}",
                    packet_loss_type,
                    packet_loss_rate * 100.0,
                    direction
                )
            }
            FaultConfiguration::Bandwidth { rate, unit, direction, side } => {
                write!(
                    f,
                    "Bandwidth Fault - Side {}, Rate: {} {}, Direction: {}",
                    side.clone().unwrap_or_default(),
                    rate,
                    unit.to_string(),
                    direction
                )
            }
            FaultConfiguration::Jitter {
                jitter_amplitude,
                jitter_frequency,
                direction,
            } => {
                write!(
                    f,
                    "Jitter Fault - Amplitude: {:.2} ms, Frequency: {:.2} Hz, Direction: {}",
                    jitter_amplitude, jitter_frequency, direction
                )
            }
            FaultConfiguration::Dns { dns_rate, direction } => {
                write!(
                    f,
                    "DNS Fault - Rate: {}%, Direction: {}",
                    dns_rate, direction
                )
            }
        }
    }
}
