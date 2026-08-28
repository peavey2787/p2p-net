package io.github.peavey2787.p2pnet

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.ContextCompat
import io.github.peavey2787.p2pnet.ui.P2PNodeApp

class MainActivity : ComponentActivity() {
    private val viewModel: MainViewModel by viewModels()

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        updatePermissionState()
        P2PNodeService.start(applicationContext)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        updatePermissionState()
        requestPermissionsThenStart()
        setContent {
            P2PNodeApp(
                viewModel = viewModel,
                onStartService = { requestPermissionsThenStart() },
                onStopService = { P2PNodeService.stop(applicationContext) },
            )
        }
    }

    override fun onLowMemory() {
        NodeRepository.trimTransientUiState()
        super.onLowMemory()
    }

    private fun requestPermissionsThenStart() {
        val missing = buildList {
            if (
                Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(
                    this@MainActivity,
                    Manifest.permission.POST_NOTIFICATIONS,
                ) != PackageManager.PERMISSION_GRANTED
            ) {
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
            if (
                Build.VERSION.SDK_INT >= 37 &&
                ContextCompat.checkSelfPermission(
                    this@MainActivity,
                    Manifest.permission.ACCESS_LOCAL_NETWORK,
                ) != PackageManager.PERMISSION_GRANTED
            ) {
                add(Manifest.permission.ACCESS_LOCAL_NETWORK)
            }
        }
        if (missing.isEmpty()) {
            updatePermissionState()
            P2PNodeService.start(applicationContext)
        } else {
            permissionLauncher.launch(missing.toTypedArray())
        }
    }

    private fun updatePermissionState() {
        val granted = if (Build.VERSION.SDK_INT >= 37) {
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.ACCESS_LOCAL_NETWORK,
            ) == PackageManager.PERMISSION_GRANTED
        } else {
            null
        }
        viewModel.updateLocalNetworkPermission(granted)
    }
}
