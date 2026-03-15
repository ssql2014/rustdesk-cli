#pragma once

#ifdef __AC_HOST_COMPILE__

__GLOBAL__ void rmsnorm_run_die_broadcast(
    void* out_ddr0, void* out_ddr1,
    void* fm_ddr0,  void* fm_ddr1,
    void* wt_ddr0,  void* wt_ddr1,
    int   seq_len, int dim_size, int dtype, float eps);

#endif

void launch_rmsnorm_broadcast_passthrough(
    void* out_ddr0,  void* out_ddr1,
    void* fm_ddr0,   void* fm_ddr1,
    void* wt_ddr0,   void* wt_ddr1,
    int   seq_len,   int   dim_size,
    int   dtype,     float eps,
    int   device_id);
