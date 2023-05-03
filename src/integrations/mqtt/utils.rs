use crate::integrations::mqtt::MqttConfig;
use crate::types::{
    device::{
        CorrelatedColorTemperature, Device, DeviceColor, DeviceId, DeviceState, Light, OnOffDevice,
        SensorKind,
    },
    integration::IntegrationId,
};
use anyhow::Result;
use json_value_merge::Merge;
use palette::Hsv;

pub fn mqtt_to_homectl(
    payload: &[u8],
    integration_id: IntegrationId,
    config: &MqttConfig,
) -> Result<Device> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;

    let id_field = config.id_field.as_deref().unwrap_or("/id");
    let name_field = config.name_field.as_deref().unwrap_or("/name");
    let color_field = config.color_field.as_deref().unwrap_or("/color");
    let cct_field = config.cct_field.as_deref().unwrap_or("/cct");
    let power_field = config.power_field.as_deref().unwrap_or("/power");
    let brightness_field = config.brightness_field.as_deref().unwrap_or("/brightness");
    let sensor_value_field = config
        .sensor_value_field
        .as_deref()
        .unwrap_or("/sensor_value");
    let transition_ms_field = config
        .transition_ms_field
        .as_deref()
        .unwrap_or("/transition_ms");

    let id = value
        .pointer(id_field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing '{}' field in MQTT message", id_field))?
        .to_string();

    let name = value
        .pointer(name_field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing '{}' field in MQTT message", name_field))?
        .to_string();

    let color = value
        .pointer(color_field)
        .and_then(|value| serde_json::from_value::<Hsv>(value.clone()).ok())
        .map(DeviceColor::Hsv)
        .or_else(|| {
            value
                .pointer(cct_field)
                .and_then(serde_json::Value::as_f64)
                .map(|value| {
                    DeviceColor::Cct(CorrelatedColorTemperature::new(
                        value as f32,
                        2700.0..6500.0,
                    ))
                })
        });

    let power = value
        .pointer(power_field)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_default();

    let brightness = value
        .pointer(brightness_field)
        .and_then(serde_json::Value::as_f64)
        .map(|value| value as f32);

    let transition_ms = value
        .pointer(transition_ms_field)
        .and_then(serde_json::Value::as_u64);

    let device_state = if value
        .pointer(sensor_value_field)
        .filter(|v| !v.is_null())
        .is_some()
    {
        if let Ok(value) = value
            .pointer(sensor_value_field)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .parse::<bool>()
        {
            DeviceState::Sensor(SensorKind::OnOffSensor { value })
        } else {
            DeviceState::Sensor(SensorKind::StringValue {
                value: value
                    .pointer(sensor_value_field)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        }
    } else if brightness.is_some() {
        DeviceState::Light(Light {
            power,
            brightness,
            color,
            transition_ms,
        })
    } else {
        DeviceState::OnOffDevice(OnOffDevice { power })
    };

    Ok(Device {
        id: DeviceId::new(&id),
        name,
        integration_id,
        scene: None,
        state: device_state,
    })
}

pub fn homectl_to_mqtt(device: Device, config: &MqttConfig) -> Result<serde_json::Value> {
    let mut payload = serde_json::Value::default();

    let id_field = config.id_field.as_deref().unwrap_or("/id");
    let name_field = config.name_field.as_deref().unwrap_or("/name");
    let color_field = config.color_field.as_deref().unwrap_or("/color");
    let cct_field = config.cct_field.as_deref().unwrap_or("/cct");
    let power_field = config.power_field.as_deref().unwrap_or("/power");
    let brightness_field = config.brightness_field.as_deref().unwrap_or("/brightness");
    let transition_ms_field = config
        .transition_ms_field
        .as_deref()
        .unwrap_or("/transition_ms");

    payload.merge_in(id_field, serde_json::Value::String(device.id.to_string()))?;
    payload.merge_in(name_field, serde_json::Value::String(device.name))?;

    match device.state {
        DeviceState::OnOffDevice(on_off_device) => {
            payload.merge_in(power_field, serde_json::Value::Bool(on_off_device.power))?;
        }
        DeviceState::Light(light) => {
            payload.merge_in(power_field, serde_json::Value::Bool(light.power))?;

            if let Some(brightness) = light.brightness {
                payload.merge_in(
                    brightness_field,
                    serde_json::Number::from_f64(brightness.into())
                        .map(serde_json::Value::Number)
                        .unwrap(),
                )?;
            }

            if let Some(DeviceColor::Hsv(hsv)) = light.color {
                payload.merge_in(color_field, serde_json::to_value(hsv)?)?;
            }

            if let Some(DeviceColor::Cct(cct)) = light.color {
                payload.merge_in(
                    cct_field,
                    serde_json::Number::from_f64(cct.get_cct().into())
                        .map(serde_json::Value::Number)
                        .unwrap(),
                )?;
            }

            if let Some(transition_ms) = light.transition_ms {
                payload.merge_in(
                    transition_ms_field,
                    serde_json::Number::from_f64(transition_ms as f64)
                        .map(serde_json::Value::Number)
                        .unwrap(),
                )?;
            }
        }
        DeviceState::MultiSourceLight(_) => unimplemented!(),
        DeviceState::Sensor(_) => unimplemented!(),
    };

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::str::FromStr;

    #[test]
    fn test_homectl_to_mqtt() {
        // Create a device and MqttConfig
        let device = Device {
            id: DeviceId::new("device1"),
            name: "Device 1".to_string(),
            integration_id: IntegrationId::from_str("mqtt").unwrap(),
            scene: None,
            state: DeviceState::Light(Light {
                power: true,
                brightness: Some(0.5),
                color: Some(DeviceColor::Hsv(Hsv::new(45.0, 1.0, 1.0))),
                transition_ms: Some(1000),
            }),
        };

        let config = MqttConfig {
            host: "localhost".to_string(),
            port: 1883,
            topic_set: "homectl/set/{id}".to_string(),
            topic: "homectl/devices/{id}".to_string(),
            id_field: Some("/id".to_string()),
            name_field: Some("/name".to_string()),
            color_field: Some("/color".to_string()),
            cct_field: Some("/cct".to_string()),
            power_field: Some("/power".to_string()),
            brightness_field: Some("/brightness".to_string()),
            sensor_value_field: Some("/sensor_value".to_string()),
            transition_ms_field: Some("/transition_ms".to_string()),
        };

        let mqtt_json = homectl_to_mqtt(device, &config).unwrap();

        let expected = json!({
            "id": "device1",
            "name": "Device 1",
            "color": { "hue": 45.0, "saturation": 1.0, "value": 1.0 },
            "power": true,
            "brightness": 0.5,
            "transition_ms": serde_json::json!(1000.0),
        });

        assert_eq!(mqtt_json, expected);
    }

    #[test]
    fn test_mqtt_to_homectl() {
        let mqtt_json = json!({
            "id": "device1",
            "name": "Device 1",
            "color": { "hue": 45.0, "saturation": 1.0, "value": 1.0 },
            "power": true,
            "brightness": 0.5,
            "transition_ms": 1000
        });

        let config = MqttConfig {
            host: "localhost".to_string(),
            port: 1883,
            topic_set: "homectl/set/{id}".to_string(),
            topic: "homectl/devices/{id}".to_string(),
            id_field: Some("/id".to_string()),
            name_field: Some("/name".to_string()),
            color_field: Some("/color".to_string()),
            cct_field: Some("/cct".to_string()),
            power_field: Some("/power".to_string()),
            brightness_field: Some("/brightness".to_string()),
            sensor_value_field: Some("/sensor_value".to_string()),
            transition_ms_field: Some("/transition_ms".to_string()),
        };

        let integration_id = IntegrationId::from_str("mqtt").unwrap();
        let device = mqtt_to_homectl(
            mqtt_json.to_string().as_bytes(),
            integration_id.clone(),
            &config,
        )
        .unwrap();

        let expected = Device {
            id: DeviceId::new("device1"),
            name: "Device 1".to_string(),
            integration_id,
            scene: None,
            state: DeviceState::Light(Light {
                power: true,
                brightness: Some(0.5),
                color: Some(DeviceColor::Hsv(Hsv::new(45.0, 1.0, 1.0))),
                transition_ms: Some(1000),
            }),
        };

        assert_eq!(device, expected);
    }

    #[tokio::test]
    async fn test_integration() {
        let mqtt_json = json!({
            "id": "device1",
            "name": "Device 1",
            "color": { "hue": 45.0, "saturation": 1.0, "value": 1.0 },
            "power": true,
            "brightness": 0.5,
        });

        let config = MqttConfig {
            host: "localhost".to_string(),
            port: 1883,
            topic_set: "homectl/set/{id}".to_string(),
            topic: "homectl/devices/{id}".to_string(),
            id_field: Some("/id".to_string()),
            name_field: Some("/name".to_string()),
            color_field: Some("/color".to_string()),
            cct_field: Some("/cct".to_string()),
            power_field: Some("/power".to_string()),
            brightness_field: Some("/brightness".to_string()),
            sensor_value_field: Some("/sensor_value".to_string()),
            transition_ms_field: Some("/transition_ms".to_string()),
        };

        let integration_id = IntegrationId::from_str("mqtt").unwrap();
        let device =
            mqtt_to_homectl(mqtt_json.to_string().as_bytes(), integration_id, &config).unwrap();
        let mqtt_message_value = homectl_to_mqtt(device, &config).unwrap();

        assert_eq!(mqtt_json, mqtt_message_value);
    }
}