package org.andromeda.link.camera

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts

/** Opens the camera app for the photo [PhoneCamera] waits on and hands back its answer; nothing of its own on screen. */
class CameraActivity : ComponentActivity() {
    private val camera =
        registerForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
            PhoneCamera.finished(intent.data, taken)
            finish()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uri = intent.data
        if (uri == null) {
            finish()
        } else if (savedInstanceState == null) {
            camera.launch(uri)
        }
    }
}
