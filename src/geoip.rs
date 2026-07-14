use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoInfo {
    pub ip: String,
    pub country: String,
    pub city: String,
    pub asn: String,
    pub isp: String,
    pub lat: f64,
    pub lon: f64,
    pub timezone: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FreeIpApiResponse {
    country_name: Option<String>,
    city_name: Option<String>,
    asn: Option<String>,
    org: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    time_zone: Option<String>,
}

const CACHE_FILE: &str = ".routescope_geoip_cache.json";

static CACHE: Lazy<Arc<Mutex<HashMap<String, GeoInfo>>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    if let Ok(mut file) = File::open(CACHE_FILE) {
        let mut content = String::new();
        if file.read_to_string(&mut content).is_ok() {
            if let Ok(loaded_map) = serde_json::from_str::<HashMap<String, GeoInfo>>(&content) {
                map = loaded_map;
            }
        }
    }
    Arc::new(Mutex::new(map))
});

fn save_cache() {
    if let Ok(map) = CACHE.lock() {
        if let Ok(serialized) = serde_json::to_string_pretty(&*map) {
            let mut tmp_file = CACHE_FILE.to_string();
            tmp_file.push_str(".tmp");
            if let Ok(mut file) = File::create(&tmp_file) {
                if file.write_all(serialized.as_bytes()).is_ok() {
                    let _ = std::fs::rename(tmp_file, CACHE_FILE);
                }
            }
        }
    }
}

pub async fn lookup_geoip(ip_str: &str) -> Option<GeoInfo> {
    // 1. Check local memory cache
    {
        if let Ok(map) = CACHE.lock() {
            if let Some(info) = map.get(ip_str) {
                return Some(info.clone());
            }
        }
    }

    // Handle local network and loopback addresses
    if crate::util::is_private_ip(ip_str) {
        let local_info = GeoInfo {
            ip: ip_str.to_string(),
            country: "Local Network".to_string(),
            city: "LAN".to_string(),
            asn: "N/A".to_string(),
            isp: "Private / Loopback".to_string(),
            lat: 0.0,
            lon: 0.0,
            timezone: "Local".to_string(),
        };
        return Some(local_info);
    }

    // 2. Check local MaxMind database if available
    if let Some(info) = lookup_local_mmdb(ip_str) {
        if let Ok(mut map) = CACHE.lock() {
            map.insert(ip_str.to_string(), info.clone());
        }
        save_cache();
        return Some(info);
    }

    // 3. Fallback to freeipapi.com
    let url = format!("https://freeipapi.com/api/json/{}", ip_str);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;

    if let Ok(response) = client.get(&url).send().await {
        if let Ok(raw) = response.json::<FreeIpApiResponse>().await {
            let asn_val = raw.asn.unwrap_or_default();
            let asn = if asn_val.is_empty() || asn_val == "0" {
                "N/A".to_string()
            } else if asn_val.starts_with("AS") {
                asn_val
            } else {
                format!("AS{}", asn_val)
            };

            let info = GeoInfo {
                ip: ip_str.to_string(),
                country: raw.country_name.unwrap_or_else(|| "Unknown".to_string()),
                city: raw.city_name.unwrap_or_else(|| "Unknown".to_string()),
                asn,
                isp: raw.org.unwrap_or_else(|| "Unknown".to_string()),
                lat: raw.latitude.unwrap_or(0.0),
                lon: raw.longitude.unwrap_or(0.0),
                timezone: raw.time_zone.unwrap_or_else(|| "UTC".to_string()),
            };

            if let Ok(mut map) = CACHE.lock() {
                map.insert(ip_str.to_string(), info.clone());
            }
            save_cache();
            return Some(info);
        }
    }

    None
}

#[derive(Deserialize)]
struct MaxMindCityRecord {
    country: Option<MaxMindCountry>,
    city: Option<MaxMindCity>,
    location: Option<MaxMindLocation>,
}

#[derive(Deserialize)]
struct MaxMindCountry {
    names: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct MaxMindCity {
    names: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct MaxMindLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
    time_zone: Option<String>,
}

fn lookup_local_mmdb(ip_str: &str) -> Option<GeoInfo> {
    let city_path = Path::new("geoip/GeoLite2-City.mmdb");
    let asn_path = Path::new("geoip/GeoLite2-ASN.mmdb");

    if !city_path.exists() {
        return None;
    }

    let ip: std::net::IpAddr = ip_str.parse().ok()?;

    let mut info = GeoInfo {
        ip: ip_str.to_string(),
        country: "Unknown".to_string(),
        city: "Unknown".to_string(),
        asn: "Unknown".to_string(),
        isp: "Unknown".to_string(),
        lat: 0.0,
        lon: 0.0,
        timezone: "UTC".to_string(),
    };

    // Parse city database
    if let Ok(reader) = maxminddb::Reader::open_readfile(city_path) {
        if let Ok(city) = reader.lookup::<MaxMindCityRecord>(ip) {
            if let Some(country) = city.country {
                if let Some(names) = country.names {
                    if let Some(name) = names.get("en") {
                        info.country = name.to_string();
                    }
                }
            }
            if let Some(city_data) = city.city {
                if let Some(names) = city_data.names {
                    if let Some(name) = names.get("en") {
                        info.city = name.to_string();
                    }
                }
            }
            if let Some(location) = city.location {
                info.lat = location.latitude.unwrap_or(0.0);
                info.lon = location.longitude.unwrap_or(0.0);
                if let Some(tz) = location.time_zone {
                    info.timezone = tz.to_string();
                }
            }
        }
    }

    // Parse ASN database
    if asn_path.exists() {
        if let Ok(reader) = maxminddb::Reader::open_readfile(asn_path) {
            #[derive(Deserialize)]
            struct AsnRecord {
                autonomous_system_number: Option<u32>,
                autonomous_system_organization: Option<String>,
            }
            if let Ok(record) = reader.lookup::<AsnRecord>(ip) {
                if let Some(asn_num) = record.autonomous_system_number {
                    info.asn = format!("AS{}", asn_num);
                }
                if let Some(asn_org) = record.autonomous_system_organization {
                    info.isp = asn_org;
                }
            }
        }
    }

    Some(info)
}
