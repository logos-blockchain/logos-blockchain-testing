use std::{collections::BTreeMap, env};

use testing_framework_core::scenario::DynError;
use testing_framework_runner_k8s::{
    BinaryConfigK8sSpec, HelmManifest, K8sBinaryApp,
    k8s_openapi::{
        api::{
            apps::v1::{Deployment, DeploymentSpec},
            core::v1::{
                ConfigMap, ConfigMapVolumeSource, Container, ContainerPort, PodSpec,
                PodTemplateSpec, Service, ServicePort, ServiceSpec, Volume, VolumeMount,
            },
        },
        apimachinery::pkg::{
            apis::meta::v1::{LabelSelector, ObjectMeta},
            util::intstr::IntOrString,
        },
    },
};

use crate::MetricsCounterEnv;

const CHART_NAME: &str = "metrics-counter";
const NODE_NAME_PREFIX: &str = "metrics-counter-node";
const NODE_CONFIG_PATH: &str = "/etc/metrics-counter/config.yaml";
const CONTAINER_HTTP_PORT: u16 = 8080;
const SERVICE_TESTING_PORT: u16 = 8081;
const PROMETHEUS_SERVICE_NAME: &str = "metrics-counter-prometheus";
const PROMETHEUS_CONTAINER_PORT: u16 = 9090;
const DEFAULT_PROMETHEUS_NODE_PORT: u16 = 30991;

impl K8sBinaryApp for MetricsCounterEnv {
    fn k8s_binary_spec() -> BinaryConfigK8sSpec {
        BinaryConfigK8sSpec::conventional(
            CHART_NAME,
            NODE_NAME_PREFIX,
            "/usr/local/bin/metrics-counter-node",
            NODE_CONFIG_PATH,
            CONTAINER_HTTP_PORT,
            SERVICE_TESTING_PORT,
        )
    }

    fn extend_k8s_manifest(
        topology: &Self::Deployment,
        manifest: &mut HelmManifest,
    ) -> Result<(), DynError> {
        manifest.extend(render_prometheus_assets(topology)?);
        Ok(())
    }
}

fn render_prometheus_assets(
    topology: &crate::MetricsCounterTopology,
) -> Result<HelmManifest, DynError> {
    let mut manifest = HelmManifest::new();
    manifest.push_yaml(&prometheus_config_map(topology))?;
    manifest.push_yaml(&prometheus_deployment())?;
    manifest.push_yaml(&prometheus_service())?;
    Ok(manifest)
}

fn prometheus_config_map(topology: &crate::MetricsCounterTopology) -> ConfigMap {
    let mut data = BTreeMap::new();
    data.insert(
        "prometheus.yml".to_owned(),
        render_prometheus_config(topology),
    );

    ConfigMap {
        metadata: ObjectMeta {
            name: Some(prometheus_config_name()),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    }
}

fn prometheus_deployment() -> Deployment {
    let labels = prometheus_labels();

    Deployment {
        metadata: ObjectMeta {
            name: Some(PROMETHEUS_SERVICE_NAME.to_owned()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "prometheus".to_owned(),
                        image: Some("prom/prometheus:v2.54.1".to_owned()),
                        args: Some(vec![
                            "--config.file=/etc/prometheus/prometheus.yml".to_owned(),
                            format!("--web.listen-address=0.0.0.0:{PROMETHEUS_CONTAINER_PORT}"),
                        ]),
                        ports: Some(vec![ContainerPort {
                            container_port: i32::from(PROMETHEUS_CONTAINER_PORT),
                            ..Default::default()
                        }]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "config".to_owned(),
                            mount_path: "/etc/prometheus/prometheus.yml".to_owned(),
                            sub_path: Some("prometheus.yml".to_owned()),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    volumes: Some(vec![Volume {
                        name: "config".to_owned(),
                        config_map: Some(ConfigMapVolumeSource {
                            name: prometheus_config_name(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn prometheus_service() -> Service {
    Service {
        metadata: ObjectMeta {
            name: Some(PROMETHEUS_SERVICE_NAME.to_owned()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(prometheus_labels()),
            type_: Some("NodePort".to_owned()),
            ports: Some(vec![ServicePort {
                name: Some("http".to_owned()),
                port: i32::from(PROMETHEUS_CONTAINER_PORT),
                target_port: Some(IntOrString::Int(i32::from(PROMETHEUS_CONTAINER_PORT))),
                node_port: Some(i32::from(prometheus_query_port())),
                protocol: Some("TCP".to_owned()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn render_prometheus_config(topology: &crate::MetricsCounterTopology) -> String {
    let targets = (0..topology.node_count)
        .map(|index| format!("\"{NODE_NAME_PREFIX}-{index}:{CONTAINER_HTTP_PORT}\""))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "global:\n  scrape_interval: 1s\nscrape_configs:\n  - job_name: metrics_counter\n    metrics_path: /metrics\n    static_configs:\n      - targets: [{targets}]\n"
    )
}

fn prometheus_query_port() -> u16 {
    env::var("METRICS_COUNTER_K8S_PROMETHEUS_NODE_PORT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PROMETHEUS_NODE_PORT)
}

fn prometheus_labels() -> BTreeMap<String, String> {
    BTreeMap::from([("app".to_owned(), PROMETHEUS_SERVICE_NAME.to_owned())])
}

fn prometheus_config_name() -> String {
    format!("{PROMETHEUS_SERVICE_NAME}-config")
}
