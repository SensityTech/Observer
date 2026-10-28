-- Deeper statistics: CPU per thread group, user and kernel time, page faults, the tree's totals, the
-- processors' clock, disk latency, steal and pressure. Every new column is NULL where it was not measured;
-- the aggregates' sample counts start at 0 for the rows already rolled up, which is what they measured.

-- FXServer's main process: the kernel part of fxs_cpu, the slot's page faults (hard ones on Linux
-- only), and the peak working set.
ALTER TABLE sample_5s ADD COLUMN fxs_cpu_kernel REAL;
ALTER TABLE sample_5s ADD COLUMN fxs_faults INTEGER;
ALTER TABLE sample_5s ADD COLUMN fxs_hard_faults INTEGER;
ALTER TABLE sample_5s ADD COLUMN fxs_ws_peak INTEGER;
-- The main process's threads by group, in percent of one core; the six are set together.
ALTER TABLE sample_5s ADD COLUMN thr_main_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN thr_network_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN thr_sync_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN thr_js_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN thr_mono_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN thr_other_cpu REAL;
-- The whole tree, ended processes included: percent of one core, processes started in the slot and the
-- peak committed memory (both Windows only).
ALTER TABLE sample_5s ADD COLUMN tree_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN tree_started INTEGER;
ALTER TABLE sample_5s ADD COLUMN tree_private_peak INTEGER;
-- The processors' clock, the highest the system states, and the share of it allowed, in percent.
ALTER TABLE sample_5s ADD COLUMN host_cpu_mhz REAL;
ALTER TABLE sample_5s ADD COLUMN host_cpu_max_mhz REAL;
ALTER TABLE sample_5s ADD COLUMN host_cpu_limit REAL;
-- Milliseconds per transfer of the data folder's volume, and the transfers in the slot.
ALTER TABLE sample_5s ADD COLUMN host_disk_ms REAL;
ALTER TABLE sample_5s ADD COLUMN host_disk_ops INTEGER;
-- Linux only: steal in percent of the machine, and the share of the slot some task stalled on each
-- resource.
ALTER TABLE sample_5s ADD COLUMN host_steal REAL;
ALTER TABLE sample_5s ADD COLUMN host_psi_cpu REAL;
ALTER TABLE sample_5s ADD COLUMN host_psi_memory REAL;
ALTER TABLE sample_5s ADD COLUMN host_psi_io REAL;

-- The same measures in the minute and hour aggregates, as the first migration lays them out.
ALTER TABLE sample_1m ADD COLUMN fxs_cpu_kernel_avg REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_cpu_kernel_min REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_cpu_kernel_max REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_cpu_kernel_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN fxs_faults_sum INTEGER;
ALTER TABLE sample_1m ADD COLUMN fxs_faults_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN fxs_hard_faults_sum INTEGER;
ALTER TABLE sample_1m ADD COLUMN fxs_hard_faults_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN fxs_ws_peak_avg REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_ws_peak_min REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_ws_peak_max REAL;
ALTER TABLE sample_1m ADD COLUMN fxs_ws_peak_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_main_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_main_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_main_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_main_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_network_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_network_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_network_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_network_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_sync_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_sync_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_sync_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_sync_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_js_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_js_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_js_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_js_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_mono_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_mono_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_mono_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_mono_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN thr_other_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN thr_other_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN thr_other_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN thr_other_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN tree_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN tree_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN tree_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN tree_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN tree_started_sum INTEGER;
ALTER TABLE sample_1m ADD COLUMN tree_started_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN tree_private_peak_avg REAL;
ALTER TABLE sample_1m ADD COLUMN tree_private_peak_min REAL;
ALTER TABLE sample_1m ADD COLUMN tree_private_peak_max REAL;
ALTER TABLE sample_1m ADD COLUMN tree_private_peak_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_cpu_mhz_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_mhz_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_mhz_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_mhz_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_cpu_max_mhz_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_max_mhz_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_max_mhz_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_max_mhz_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_cpu_limit_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_limit_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_limit_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_cpu_limit_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_disk_ms_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_disk_ms_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_disk_ms_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_disk_ms_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_disk_ops_sum INTEGER;
ALTER TABLE sample_1m ADD COLUMN host_disk_ops_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_steal_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_steal_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_steal_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_steal_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_psi_cpu_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_cpu_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_cpu_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_psi_memory_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_memory_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_memory_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_memory_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1m ADD COLUMN host_psi_io_avg REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_io_min REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_io_max REAL;
ALTER TABLE sample_1m ADD COLUMN host_psi_io_n INTEGER NOT NULL DEFAULT 0;

ALTER TABLE sample_1h ADD COLUMN fxs_cpu_kernel_avg REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_cpu_kernel_min REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_cpu_kernel_max REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_cpu_kernel_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN fxs_faults_sum INTEGER;
ALTER TABLE sample_1h ADD COLUMN fxs_faults_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN fxs_hard_faults_sum INTEGER;
ALTER TABLE sample_1h ADD COLUMN fxs_hard_faults_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN fxs_ws_peak_avg REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_ws_peak_min REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_ws_peak_max REAL;
ALTER TABLE sample_1h ADD COLUMN fxs_ws_peak_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_main_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_main_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_main_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_main_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_network_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_network_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_network_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_network_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_sync_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_sync_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_sync_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_sync_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_js_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_js_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_js_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_js_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_mono_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_mono_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_mono_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_mono_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN thr_other_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN thr_other_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN thr_other_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN thr_other_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN tree_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN tree_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN tree_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN tree_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN tree_started_sum INTEGER;
ALTER TABLE sample_1h ADD COLUMN tree_started_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN tree_private_peak_avg REAL;
ALTER TABLE sample_1h ADD COLUMN tree_private_peak_min REAL;
ALTER TABLE sample_1h ADD COLUMN tree_private_peak_max REAL;
ALTER TABLE sample_1h ADD COLUMN tree_private_peak_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_cpu_mhz_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_mhz_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_mhz_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_mhz_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_cpu_max_mhz_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_max_mhz_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_max_mhz_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_max_mhz_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_cpu_limit_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_limit_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_limit_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_cpu_limit_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_disk_ms_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_disk_ms_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_disk_ms_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_disk_ms_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_disk_ops_sum INTEGER;
ALTER TABLE sample_1h ADD COLUMN host_disk_ops_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_steal_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_steal_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_steal_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_steal_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_psi_cpu_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_cpu_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_cpu_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_cpu_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_psi_memory_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_memory_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_memory_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_memory_n INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sample_1h ADD COLUMN host_psi_io_avg REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_io_min REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_io_max REAL;
ALTER TABLE sample_1h ADD COLUMN host_psi_io_n INTEGER NOT NULL DEFAULT 0;

-- Thread names, which are not a fixed set.
CREATE TABLE thread_name (
    id   INTEGER PRIMARY KEY,
    name TEXT    NOT NULL UNIQUE
) STRICT;

-- The busiest thread names of a slot: percent of one core, that name's threads together.
CREATE TABLE thread_5s (
    ts   INTEGER NOT NULL,
    name INTEGER NOT NULL REFERENCES thread_name (id),
    cpu  REAL    NOT NULL,
    PRIMARY KEY (ts, name)
) STRICT, WITHOUT ROWID;
