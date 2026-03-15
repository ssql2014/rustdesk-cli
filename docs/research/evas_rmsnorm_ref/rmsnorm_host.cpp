#include <hpe.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "BuildConfig.h"
#include "EvModuleManager.h"
#include "rmsnorm.h"

void launch_rmsnorm_die_passthrough(void* out_ddr0, void* output_res_ddr0, void* fm_ddr0, void* wt_ddr0,
                                        void* in_res_ddr0, float eps, int seq_len0, int dim_size0,
                                        int dtype, int device_id, int m_tile_size = 16) {
  OPS_PRINT("[Host Launcher] pointers:\n");
  OPS_PRINT("              - die0_out_ptr:  %p\n", out_ddr0);
  OPS_PRINT("              - die0_output_res_ptr: %p\n", output_res_ddr0);
  OPS_PRINT("              - die0_in_ptr:   %p\n", fm_ddr0);
  OPS_PRINT("              - die0_wt_ptr:   %p\n", wt_ddr0);
  OPS_PRINT("              - die0_in_res_ptr: %p\n", in_res_ddr0);
  OPS_PRINT_FLUSH();

  evSetDevice(device_id);

  evModule_t module = EvModuleManager::getInstance().getModule();
  if (module == nullptr) {
    fprintf(stderr, "Error: failed to get module from EvModuleManager.\n");
    return;
  }

  evError_t err = evConfigureCall(4, 4);
  if (err != evSuccess) {
    fprintf(stderr, "Error: evConfigureCall failed, error: %d\n", err);
    return;
  }

  void* args[] = {
      &out_ddr0,    (void*)8, &output_res_ddr0, (void*)8, &fm_ddr0,   (void*)8, &wt_ddr0,  (void*)8,
      &in_res_ddr0, (void*)8, &seq_len0,        (void*)8, &dim_size0, (void*)8, &dtype,     (void*)8,
      &eps,         (void*)8,
      NULL
  };

  OPS_PRINT("[Host Launcher] launching kernel 'rmsnorm_run_die'...\n");
  err = evLaunchKernel(&module, "rmsnorm_run_die", args);
  if (err != evSuccess) {
    fprintf(stderr, "Error: evLaunchKernel failed, error: %d\n", err);
    return;
  }

  evDeviceSynchronize();
  OPS_PRINT("[Host Launcher] kernel done.\n");
}
