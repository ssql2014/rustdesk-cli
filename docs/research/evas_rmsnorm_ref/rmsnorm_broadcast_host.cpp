#include <hpe.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "BuildConfig.h"
#include "EvModuleManager.h"
#include "rmsnorm_broadcast.h"

void launch_rmsnorm_broadcast_passthrough(
    void* out_ddr0,  void* out_ddr1,
    void* fm_ddr0,   void* fm_ddr1,
    void* wt_ddr0,   void* wt_ddr1,
    int   seq_len,   int   dim_size,
    int   dtype,     float eps,
    int   device_id)
{
    OPS_PRINT("[rmsnorm_broadcast Host] launching kernel:\n");
    OPS_PRINT("  out_ddr0=%p  out_ddr1=%p\n", out_ddr0, out_ddr1);
    OPS_PRINT("  fm_ddr0=%p   fm_ddr1=%p\n",  fm_ddr0,  fm_ddr1);
    OPS_PRINT("  wt_ddr0=%p   wt_ddr1=%p\n",  wt_ddr0,  wt_ddr1);
    OPS_PRINT("  seq_len=%d   dim_size=%d  dtype=%d  eps=%f\n",
              seq_len, dim_size, dtype, eps);
    OPS_PRINT_FLUSH();

    evError_t err = evSetDevice(device_id);
    if (err != evSuccess) {
        fprintf(stderr, "[rmsnorm_broadcast] evSetDevice failed, error: %d\n", err);
        return;
    }

    evModule_t module = EvModuleManager::getInstance().getModule();
    if (module == nullptr) {
        printf("[rmsnorm_broadcast] module not ready, trying vllm_fusedop_init...\n");
        evConfigureCall(8, 4);
        void* init_args[] = { NULL };
        evLaunchKernel(nullptr, "vllm_fusedop_init", init_args);
        evDeviceSynchronize();

        module = EvModuleManager::getInstance().getModule();
        if (module == nullptr) {
            fprintf(stderr, "[rmsnorm_broadcast] fatal: module still null\n");
            return;
        }
    }

    err = evConfigureCall(8, 4);
    if (err != evSuccess) {
        fprintf(stderr, "[rmsnorm_broadcast] evConfigureCall failed, error: %d\n", err);
        return;
    }

    void* args[] = {
        &out_ddr0,  (void*)8,
        &out_ddr1,  (void*)8,
        &fm_ddr0,   (void*)8,
        &fm_ddr1,   (void*)8,
        &wt_ddr0,   (void*)8,
        &wt_ddr1,   (void*)8,
        &seq_len,   (void*)8,
        &dim_size,  (void*)8,
        &dtype,     (void*)8,
        &eps,       (void*)8,
        NULL
    };

    OPS_PRINT("[rmsnorm_broadcast Host] launching 'rmsnorm_run_die_broadcast'...\n");
    err = evLaunchKernel(&module, "rmsnorm_run_die_broadcast", args);
    if (err != evSuccess) {
        fprintf(stderr, "[rmsnorm_broadcast] evLaunchKernel failed, error: %d\n", err);
        return;
    }

    err = evDeviceSynchronize();
    if (err != evSuccess) {
        fprintf(stderr, "[rmsnorm_broadcast] evDeviceSynchronize failed, error: %d\n", err);
        return;
    }
    OPS_PRINT("[rmsnorm_broadcast Host] kernel done.\n");
}
