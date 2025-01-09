use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::cli::CliBandwidthConfig;
use crate::cli::CliDNSConfig;
use crate::cli::CliJitterConfig;
use crate::cli::CliLatencyConfig;
use crate::cli::CliPacketLossConfig;
use crate::errors::ProxyError;
use crate::types::BandwidthUnit;
use crate::types::Direction;
use crate::types::LatencyDistribution;
use crate::types::PacketLossType;

/// Internal Configuration for Latency Fault
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct LatencySettings {
    pub distribution: LatencyDistribution,
    pub direction: Direction,
    pub latency_mean: f64,
    pub latency_stddev: f64,
    pub latency_shape: f64,
    pub latency_scale: f64,
    pub latency_min: f64,
    pub latency_max: f64,
}

/// Internal Configuration for Packet Loss Fault
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct PacketLossSettings {
    pub direction: Direction,
    pub loss_type: PacketLossType,
    pub packet_loss_rate: f64,
}

/// Internal Configuration for Bandwidth Throttling Fault
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct BandwidthSettings {
    pub direction: Direction,
    pub bandwidth_rate: u32, // in bits per second
    pub bandwidth_unit: BandwidthUnit,
}

/// Internal Configuration for Jitter Fault
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct JitterSettings {
    pub direction: Direction,
    pub jitter_amplitude: f64, // in milliseconds
    pub jitter_frequency: f64, // in Hertz
}

/// Internal Configuration for DNS Fault
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct DnsSettings {
    pub direction: Direction,
    pub dns_rate: u8, // between 0 and 100
}

/// Fault Configuration Enum
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum FaultConfig {
    Dns(DnsSettings),
    Latency(LatencySettings),
    PacketLoss(PacketLossSettings),
    Bandwidth(BandwidthSettings),
    Jitter(JitterSettings),
}

/// Implement Default manually for FaultConfig
impl Default for FaultConfig {
    fn default() -> Self {
        FaultConfig::Latency(LatencySettings::default())
    }
}

impl fmt::Display for FaultConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaultConfig::Dns(_) => write!(f, "dns"),
            FaultConfig::Latency(_) => write!(f, "latency"),
            FaultConfig::PacketLoss(_) => write!(f, "packet-loss"),
            FaultConfig::Bandwidth(_) => write!(f, "bandwidth"),
            FaultConfig::Jitter(_) => write!(f, "jitter"),
        }
    }
}

/// Proxy Configuration Struct
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct ProxyConfig {
    pub fault: FaultConfig,
}

impl ProxyConfig {
    pub fn new(fault_config: FaultConfig) -> Result<Self, ProxyError> {
        // Validation logic
        match &fault_config {
            FaultConfig::Dns(config) => {
                if config.dns_rate > 100 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Probability to trigger the DNS failure must be below 100.".into(),
                    ));
                }
            }
            FaultConfig::Latency(config) => {
                if config.latency_mean <= 0.0 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Latency mean must be a positive value.".into(),
                    ));
                }
                if config.latency_stddev < 0.0 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Latency standard deviation must be non-negative."
                            .into(),
                    ));
                }
            }
            FaultConfig::PacketLoss(config) => {
                if !(0.0..=1.0).contains(&config.packet_loss_rate) {
                    return Err(ProxyError::InvalidConfiguration(
                        "Packet loss rate must be between 0.0 and 1.0.".into(),
                    ));
                }
            }
            FaultConfig::Bandwidth(config) => {
                if config.bandwidth_rate == 0 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Bandwidth rate must be a positive integer.".into(),
                    ));
                }
            }
            FaultConfig::Jitter(config) => {
                if config.jitter_amplitude < 0.0 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Jitter amplitude must be non-negative.".into(),
                    ));
                }
                if config.jitter_frequency < 0.0 {
                    return Err(ProxyError::InvalidConfiguration(
                        "Jitter frequency must be non-negative.".into(),
                    ));
                }
            }
        }

        Ok(Self { fault: fault_config })
    }

    pub fn new_dns(
        config: CliDNSConfig,
        direction: Direction,
    ) -> Result<Self, String> {
        let dns_settings = DnsSettings { dns_rate: config.rate, direction };
        let fault_config = FaultConfig::Dns(dns_settings);
        Ok(Self { fault: fault_config })
    }

    pub fn new_latency(
        config: CliLatencyConfig,
        direction: Direction,
    ) -> Result<Self, String> {
        let latency_settings = LatencySettings {
            distribution: config.distribution,
            latency_mean: config.mean,
            latency_stddev: config.stddev,
            latency_min: config.min,
            latency_max: config.max,
            latency_shape: config.shape,
            latency_scale: config.scale,
            direction,
        };
        let fault_config = FaultConfig::Latency(latency_settings);
        Ok(Self { fault: fault_config })
    }

    pub fn new_packet_loss(
        config: CliPacketLossConfig,
        direction: Direction,
    ) -> Result<Self, String> {
        let packet_loss_settings = PacketLossSettings {
            loss_type: config.loss_type,
            packet_loss_rate: config.rate,
            direction,
        };
        let fault_config = FaultConfig::PacketLoss(packet_loss_settings);
        Ok(Self { fault: fault_config })
    }

    pub fn new_bandwidth(
        config: CliBandwidthConfig,
        direction: Direction,
    ) -> Result<Self, String> {
        let bandwidth_settings = BandwidthSettings {
            bandwidth_rate: config.rate,
            bandwidth_unit: config.unit,
            direction,
        };
        let fault_config = FaultConfig::Bandwidth(bandwidth_settings);
        Ok(Self { fault: fault_config })
    }

    pub fn new_jitter(
        config: CliJitterConfig,
        direction: Direction,
    ) -> Result<Self, String> {
        let jitter_settings = JitterSettings {
            jitter_amplitude: config.amplitude,
            jitter_frequency: config.frequency,
            direction,
        };
        let fault_config = FaultConfig::Jitter(jitter_settings);
        Ok(Self { fault: fault_config })
    }
}
