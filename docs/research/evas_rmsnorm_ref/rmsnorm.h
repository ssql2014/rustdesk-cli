#pragma once

#ifdef __AC_HOST_COMPILE__

__GLOBAL__ void rmsnorm_run_die(void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0,
                                void* in_res_ddr0, int seq_len0, int dim_size0, int dtype = 1,
                                float eps = 1e-5);

__GLOBAL__ void rmsnorm_run_die_v2(void* out_ddr0, void* output_res_ddr0, void* fm_ddr0,
                                   void* wt_ddr0, void* in_res_ddr0, void* out_ddr1,
                                   void* output_res_ddr1, void* fm_ddr1, void* wt_ddr1,
                                   void* in_res_ddr1, int seq_len, int dim_size, int dtype = 1,
                                   float eps = 1e-5);

#endif

void launch_rmsnorm_passthrough(void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0,
                                void* in_res_ddr0, float eps, int seq_len0, int dim_size0,
                                int dtype, int device_id, int m_tile_size = 16);

void launch_rmsnorm_die_passthrough(void* out_ddr0, void* output_res_ddr0, void* fm_ddr0,
                                    void* wt_ddr0, void* in_res_ddr0, float eps, int seq_len0,
                                    int dim_size0, int dtype, int device_id, int m_tile_size = 16);
