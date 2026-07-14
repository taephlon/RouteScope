use crate::stats::HopStats;
use serde_json::json;
use std::fs::File;
use std::io::Write;

pub fn export_json(path: &str, stats: &[HopStats]) -> Result<(), String> {
    let mut writer: Box<dyn Write> = if path == "stdout" {
        Box::new(std::io::stdout())
    } else {
        let file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
        Box::new(file)
    };

    serde_json::to_writer_pretty(&mut writer, stats)
        .map_err(|e| format!("Failed to write JSON: {}", e))?;

    if path == "stdout" {
        let _ = writer.write_all(b"\n");
    }
    Ok(())
}

pub fn export_csv(path: &str, stats: &[HopStats]) -> Result<(), String> {
    let mut writer: Box<dyn Write> = if path == "stdout" {
        Box::new(std::io::stdout())
    } else {
        let file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
        Box::new(file)
    };

    let mut wtr = csv::Writer::from_writer(&mut writer);

    wtr.write_record([
        "Hop",
        "IP",
        "Hostname",
        "Country",
        "City",
        "ASN",
        "ISP",
        "Sent",
        "Recv",
        "Loss%",
        "Last_RTT_ms",
        "Avg_RTT_ms",
        "Min_RTT_ms",
        "Max_RTT_ms",
        "Jitter_ms",
    ])
    .map_err(|e| format!("Failed to write CSV header: {}", e))?;

    for hop in stats {
        let ip_str = hop.ip.map(|ip| ip.to_string()).unwrap_or_default();
        let host_str = hop.hostname.clone().unwrap_or_default();

        let (country, city, asn, isp) = if let Some(ref geo) = hop.geo_info {
            (
                geo.country.as_str(),
                geo.city.as_str(),
                geo.asn.as_str(),
                geo.isp.as_str(),
            )
        } else {
            ("", "", "", "")
        };

        wtr.write_record(&[
            hop.hop_num.to_string(),
            ip_str,
            host_str,
            country.to_string(),
            city.to_string(),
            asn.to_string(),
            isp.to_string(),
            hop.sent.to_string(),
            hop.recv.to_string(),
            format!("{:.1}%", hop.loss_pct()),
            hop.last_rtt
                .map(|r| format!("{:.2}", r))
                .unwrap_or_default(),
            format!("{:.2}", hop.avg_rtt()),
            format!("{:.2}", hop.min_rtt()),
            format!("{:.2}", hop.max_rtt()),
            format!("{:.2}", hop.jitter()),
        ])
        .map_err(|e| format!("Failed to write CSV record: {}", e))?;
    }

    wtr.flush()
        .map_err(|e| format!("Failed to flush CSV: {}", e))?;
    std::mem::drop(wtr);
    if path == "stdout" {
        let _ = writer.write_all(b"\n");
    }
    Ok(())
}

pub fn export_geojson(path: &str, stats: &[HopStats]) -> Result<(), String> {
    let mut features = Vec::new();
    let mut coordinates = Vec::new();

    // 1. Add Point features for each hop with coordinates
    for hop in stats {
        if let Some(ref geo) = hop.geo_info {
            if geo.lat != 0.0 || geo.lon != 0.0 {
                coordinates.push(vec![geo.lon, geo.lat]);

                let ip_str = hop.ip.map(|ip| ip.to_string()).unwrap_or_default();
                let host_str = hop.hostname.clone().unwrap_or_default();

                let feature = json!({
                    "type": "Feature",
                    "geometry": {
                        "type": "Point",
                        "coordinates": [geo.lon, geo.lat]
                    },
                    "properties": {
                        "hop": hop.hop_num,
                        "ip": ip_str,
                        "hostname": host_str,
                        "country": geo.country,
                        "city": geo.city,
                        "asn": geo.asn,
                        "isp": geo.isp,
                        "avg_rtt_ms": hop.avg_rtt(),
                        "loss_pct": hop.loss_pct()
                    }
                });
                features.push(feature);
            }
        }
    }

    // 2. Add LineString feature connecting all the hops
    if coordinates.len() >= 2 {
        let route_feature = json!({
            "type": "Feature",
            "geometry": {
                "type": "LineString",
                "coordinates": coordinates
            },
            "properties": {
                "name": "Trace Route Path"
            }
        });
        features.push(route_feature);
    }

    let geojson = json!({
        "type": "FeatureCollection",
        "features": features
    });

    let mut writer: Box<dyn Write> = if path == "stdout" {
        Box::new(std::io::stdout())
    } else {
        let file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
        Box::new(file)
    };

    let serialized = serde_json::to_string_pretty(&geojson)
        .map_err(|e| format!("Failed to serialize GeoJSON: {}", e))?;
    writer
        .write_all(serialized.as_bytes())
        .map_err(|e| format!("Failed to write GeoJSON: {}", e))?;

    if path == "stdout" {
        let _ = writer.write_all(b"\n");
    }
    Ok(())
}
