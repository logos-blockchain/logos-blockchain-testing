# Node Metrics Blueprint

Prometheus-friendly metrics organized by domain. Use low-cardinality labels: `node_id`, `role` (validator/executor), `network_id`, plus `service/component`, `direction`, `status/type`, `endpoint/op` (coarse). Avoid per-peer labels on hot paths.

## Consensus
- Gauges: `consensus_height`, `consensus_finalized_height`, `consensus_round`, `consensus_active_validators`, `consensus_forks`, `consensus_lag_blocks`.
- Counters: `consensus_blocks_proposed_total`, `consensus_blocks_committed_total`, `consensus_blocks_rejected_total`, `consensus_view_changes_total`, `consensus_equivocation_events_total`, `consensus_leader_election_rounds_total`, `consensus_fork_detections_total`.
- Histograms: `consensus_block_production_duration_seconds`, `consensus_block_validation_duration_seconds`, `consensus_finality_time_seconds`, `consensus_fork_resolution_duration_seconds`, `consensus_message_size_bytes`, `consensus_message_latency_seconds`.
- Derived: `consensus_participation_rate`.

## Data Availability (DA)
- Gauges: `da_slot_height`, `da_storage_utilization_bytes`, `da_replication_factor`.
- Counters: `da_blobs_committed_total`, `da_blob_failures_total{reason}`, `da_sampling_requests_total`, `da_sampling_success_total`, `da_reconstruction_success_total`, `da_reconstruction_failures_total`, `da_erasure_coding_failures_total`.
- Histograms: `da_blob_dispersal_duration_seconds`, `da_blob_reconstruction_duration_seconds`, `da_sampling_duration_seconds`, `da_kzg_proof_generation_duration_seconds`, `da_kzg_proof_verification_duration_seconds`, `da_download_duration_seconds`, `da_network_message_size_bytes`.
- Derived: `da_availability_rate`, `da_sampling_success_rate`, `da_reconstruction_success_rate`.

## Blend / Subnet Balancer
- Gauges: `blend_subnets_total`, `blend_active_subnets`, `blend_peer_assignments`, `blend_anonymity_set_size` (if measurable).
- Counters: `blend_rebalances_total`, `blend_subnet_membership_changes_total`, `blend_subnet_join_failures_total`, `blend_cover_traffic_messages_total`.
- Histograms: `blend_rebalance_duration_seconds`, `blend_message_mix_duration_seconds`, `blend_layer_processing_duration_seconds`, `blend_zk_proof_generation_duration_seconds`.

## Networking (per service: libp2p, consensus, DA, API transport)
- Gauges: `network_peer_count{service}`, `network_connections{service,state}`, `network_dials_inflight{service}`.
- Counters: `network_messages_total{service,direction,type}`, `network_bytes_total{service,direction}`, `network_connect_failures_total{service,reason}`, `network_dial_attempts_total{service}`.
- Histograms: `network_message_size_bytes{service}`, `network_message_latency_seconds{service}`, `network_connection_duration_seconds{service}`, `network_rtt_seconds` (sampled).

## Storage
- Gauges: `storage_db_size_bytes{db}`, `storage_column_size_bytes{db,column}` (if cheap), `storage_state_size_bytes`, `storage_cache_hit_ratio`.
- Counters: `storage_ops_total{db,op}`, `storage_errors_total{db,op,reason}`, `storage_compactions_total{db}`.
- Histograms: `storage_op_duration_seconds{db,op}`, `storage_compaction_duration_seconds{db}`.

## Mempool / Transactions
- Gauges: `mempool_tx_count`, `mempool_size_bytes`, `mempool_utilization_ratio`.
- Counters: `mempool_txs_submitted_total`, `mempool_txs_committed_total`, `mempool_txs_rejected_total{reason}`, `mempool_txs_evicted_total{reason}`, `mempool_txs_broadcast_total`.
- Histograms: `mempool_enqueue_duration_seconds`, `mempool_tx_lifetime_seconds`, `tx_validation_duration_seconds`, `tx_processing_duration_seconds`.
- Derived: `tx_throughput_tps`, `tx_rejection_rate`, `tx_latency_seconds` (end-to-end).

## API (HTTP/gRPC)
- Counters: `api_requests_total{method,endpoint,status_class}`, `api_errors_total{endpoint,reason}`, `api_auth_failures_total`.
- Histograms: `api_request_duration_seconds{endpoint}`, `api_request_size_bytes{endpoint}`, `api_response_size_bytes{endpoint}`.
- Gauges: `api_concurrent_connections`.

## General Node Health
- Gauges/counters: `node_uptime_seconds` (counter), `node_restart_total`, `node_cpu_seconds_total`, `node_memory_bytes`, `node_threads`, `node_fd_used`, `node_logical_disk_usage_bytes`.

## Cross-Domain Signals
- Counters: `consensus_blocks_missing_da_total`, `consensus_blocks_missing_witness_total`, `da_proof_verification_failures_total`.
- Gauges: `consensus_sync_catchup_in_progress` (0/1).

## Label Guidance
- Core: `node_id`, `role` (validator/executor), `network_id`, `version`.
- Domain: `service`/`component`, `direction` (ingress/egress), `status_class` (2xx/4xx/5xx), `op`/`endpoint`, `reason` (coarse buckets).
- Avoid per-peer labels on hot metrics; aggregate where possible.
