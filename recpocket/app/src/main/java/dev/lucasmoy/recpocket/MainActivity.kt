package dev.lucasmoy.recpocket

import android.Manifest
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings as AndroidSettings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The whole app's UI: what to record, how small, when to start by itself,
 * and the buttons to do it now.
 *
 * Deliberately one screen. The app has one job and four choices, and a
 * settings hierarchy would be scaffolding around them. The recording itself
 * lives in [CaptureService] -- closing this screen must not stop a
 * recording, which is the point of the floating button and the notification.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { Screen() }
    }

    @Composable
    private fun Screen() {
        val context = LocalContext.current
        val settings = remember { Settings(context) }
        var sources by remember { mutableStateOf(settings.sources) }
        var screen by remember { mutableStateOf(settings.screen) }
        var audio by remember { mutableStateOf(settings.audioQuality) }
        var video by remember { mutableStateOf(settings.videoQuality) }
        var trigger by remember { mutableStateOf(settings.callTrigger) }
        var overlay by remember { mutableStateOf(settings.overlay) }
        var micGranted by remember { mutableStateOf(hasMic()) }

        val armed by CaptureService.isArmed.collectAsState()
        val recording by CaptureService.isRecording.collectAsState()
        val lastResult by CaptureService.lastResult.collectAsState()

        val micLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission()
        ) { granted -> micGranted = granted }

        val notificationsLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission()
        ) { }

        // Screen capture is a consent dialog, and its result is what the
        // service needs. Asked for once and kept while armed -- which is the
        // only way an automatic recording can start during a call, since an
        // app cannot raise this dialog from the background.
        val projectionLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) { result ->
            val data = result.data
            if (result.resultCode == RESULT_OK && data != null) {
                startForegroundService(
                    Intent(this, CaptureService::class.java)
                        .setAction(CaptureService.ACTION_ARM)
                        .putExtra(CaptureService.EXTRA_RESULT_CODE, result.resultCode)
                        .putExtra(CaptureService.EXTRA_RESULT_DATA, data)
                )
            }
        }

        androidx.compose.runtime.LaunchedEffect(Unit) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
                android.content.pm.PackageManager.PERMISSION_GRANTED
            ) {
                notificationsLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }

        AppTheme {
            Surface(modifier = Modifier.fillMaxSize()) {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 14.dp),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(top = 16.dp, bottom = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            stringResource(R.string.app_name),
                            fontSize = 22.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            "v${BuildConfig.VERSION_NAME}",
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    // ---- record now ------------------------------------
                    Card(Modifier.fillMaxWidth()) {
                        Column(Modifier.padding(12.dp)) {
                            if (!micGranted) {
                                Text(stringResource(R.string.mic_missing), fontSize = 13.sp)
                                OutlinedButton(
                                    modifier = Modifier.padding(top = 6.dp),
                                    onClick = { micLauncher.launch(Manifest.permission.RECORD_AUDIO) },
                                ) { Text(stringResource(R.string.action_mic)) }
                            }
                            Text(
                                if (recording) {
                                    stringResource(R.string.state_recording, "")
                                } else if (armed) {
                                    stringResource(R.string.state_armed)
                                } else {
                                    stringResource(R.string.state_idle)
                                },
                                fontSize = 13.sp,
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(
                                stringResource(R.string.arm_hint),
                                fontSize = 12.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
                            )
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                if (!armed) {
                                    OutlinedButton(onClick = {
                                        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE)
                                            as MediaProjectionManager
                                        projectionLauncher.launch(manager.createScreenCaptureIntent())
                                    }) { Text(stringResource(R.string.action_arm)) }
                                }
                                Button(
                                    enabled = micGranted,
                                    onClick = {
                                        if (recording) {
                                            CaptureService.send(context, CaptureService.ACTION_STOP)
                                        } else {
                                            CaptureService.send(context, CaptureService.ACTION_START)
                                        }
                                    },
                                ) {
                                    Text(
                                        stringResource(
                                            if (recording) R.string.action_stop else R.string.action_record
                                        )
                                    )
                                }
                                if (armed) {
                                    OutlinedButton(onClick = {
                                        CaptureService.send(context, CaptureService.ACTION_SHOT)
                                    }) { Text(stringResource(R.string.action_screenshot)) }
                                }
                            }
                            lastResult?.let { result ->
                                Row(
                                    Modifier.padding(top = 10.dp).fillMaxWidth(),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text(
                                        result.error ?: result.name,
                                        fontSize = 12.sp,
                                        color = if (result.error != null) {
                                            MaterialTheme.colorScheme.error
                                        } else {
                                            MaterialTheme.colorScheme.onSurfaceVariant
                                        },
                                        modifier = Modifier.weight(1f),
                                    )
                                    result.uri?.let { uri ->
                                        TextButton(onClick = { open(uri) }) {
                                            Text(stringResource(R.string.action_open))
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ---- what ------------------------------------------
                    Section(stringResource(R.string.section_what)) {
                        listOf(
                            Settings.Sources.MIC to R.string.source_mic,
                            Settings.Sources.PLAYBACK to R.string.source_playback,
                            Settings.Sources.BOTH to R.string.source_both,
                        ).forEach { (value, label) ->
                            Choice(stringResource(label), sources == value) {
                                sources = value
                                settings.sources = value
                            }
                        }
                        Toggle(stringResource(R.string.include_screen), screen) {
                            screen = it
                            settings.screen = it
                        }
                    }

                    // ---- quality ---------------------------------------
                    Section(stringResource(R.string.section_quality)) {
                        Text(
                            stringResource(R.string.quality_audio),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Settings.AudioQuality.entries.forEach { value ->
                            Choice("${value.label} · ${value.bitrate / 1000} kbps", audio == value) {
                                audio = value
                                settings.audioQuality = value
                            }
                        }
                        Text(
                            stringResource(R.string.quality_video),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.padding(top = 8.dp),
                        )
                        Settings.VideoQuality.entries.forEach { value ->
                            Choice("${value.label} · ${value.bitrate / 1_000_000f} Mbps", video == value) {
                                video = value
                                settings.videoQuality = value
                            }
                        }
                        Text(
                            stringResource(R.string.quality_hint),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 6.dp),
                        )
                    }

                    // ---- automatic -------------------------------------
                    Section(stringResource(R.string.section_trigger)) {
                        Toggle(stringResource(R.string.trigger_calls), trigger) {
                            trigger = it
                            settings.callTrigger = it
                            // Asked for only when the trigger is switched
                            // on: notification access is a large permission
                            // and there is no reason to hold it otherwise.
                            if (it && !hasNotificationAccess()) openNotificationAccess()
                        }
                        Text(
                            stringResource(R.string.trigger_hint),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (trigger && !hasNotificationAccess()) {
                            OutlinedButton(
                                modifier = Modifier.padding(top = 6.dp),
                                onClick = { openNotificationAccess() },
                            ) { Text(stringResource(R.string.trigger_grant)) }
                        }
                    }

                    // ---- floating button -------------------------------
                    Section(stringResource(R.string.section_floating)) {
                        Toggle(stringResource(R.string.floating_show), overlay) {
                            overlay = it
                            settings.overlay = it
                            if (it && Overlay.canShow(context)) Overlay.show(context)
                            if (!it) Overlay.hide(context)
                        }
                        Text(
                            stringResource(R.string.floating_hint),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (overlay && !Overlay.canShow(context)) {
                            OutlinedButton(
                                modifier = Modifier.padding(top = 6.dp),
                                onClick = {
                                    startActivity(
                                        Intent(
                                            AndroidSettings.ACTION_MANAGE_OVERLAY_PERMISSION,
                                            Uri.parse("package:$packageName"),
                                        )
                                    )
                                },
                            ) { Text(stringResource(R.string.floating_grant)) }
                        }
                    }

                    UpdateSection()
                }
            }
        }
    }

    @Composable
    private fun Section(title: String, content: @Composable () -> Unit) {
        Card(Modifier.fillMaxWidth().padding(top = 12.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(title, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                content()
            }
        }
    }

    @Composable
    private fun Choice(label: String, selected: Boolean, onPick: () -> Unit) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RadioButton(selected = selected, onClick = onPick)
            Text(label, fontSize = 13.sp)
        }
    }

    @Composable
    private fun Toggle(label: String, value: Boolean, onChange: (Boolean) -> Unit) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(label, fontSize = 13.sp, modifier = Modifier.weight(1f))
            Switch(checked = value, onCheckedChange = onChange)
        }
    }

    /** The same in-app update flow the sibling apps have. */
    @Composable
    private fun UpdateSection() {
        val context = LocalContext.current
        var update by remember { mutableStateOf<Updater.Available?>(null) }
        var busy by remember { mutableStateOf(false) }
        var progress by remember { mutableFloatStateOf(-1f) }
        var status by remember { mutableStateOf("") }

        Card(Modifier.fillMaxWidth().padding(top = 12.dp, bottom = 24.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(
                    stringResource(R.string.section_updates),
                    fontSize = 12.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    stringResource(R.string.update_current, BuildConfig.VERSION_NAME),
                    fontSize = 13.sp,
                    modifier = Modifier.padding(top = 6.dp),
                )
                Text(
                    stringResource(R.string.update_hint),
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(
                    Modifier.padding(top = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedButton(
                        enabled = !busy,
                        onClick = {
                            busy = true
                            status = getString(R.string.update_checking)
                            lifecycleScope.launch {
                                val result = withContext(Dispatchers.IO) {
                                    Updater.check(BuildConfig.VERSION_NAME)
                                }
                                busy = false
                                result.fold(
                                    onSuccess = { available ->
                                        update = available
                                        status = when {
                                            available.hasUpdate && available.apkUrl.isNotEmpty() ->
                                                getString(R.string.update_available, available.latest)
                                            available.hasUpdate ->
                                                getString(R.string.update_no_apk, available.latest)
                                            else -> getString(R.string.update_none, available.current)
                                        }
                                    },
                                    onFailure = {
                                        status = getString(R.string.update_failed, it.message ?: "")
                                    },
                                )
                            }
                        },
                    ) { Text(stringResource(R.string.action_check_updates)) }

                    val available = update
                    if (available != null && available.hasUpdate && available.apkUrl.isNotEmpty()) {
                        Button(
                            enabled = !busy,
                            onClick = {
                                // Permission first: downloading 10MB and
                                // then discovering the install is blocked is
                                // the worst possible order.
                                if (!Updater.canInstall(context)) {
                                    status = getString(R.string.update_needs_permission)
                                    startActivity(Updater.installPermissionIntent(context))
                                    return@Button
                                }
                                busy = true
                                progress = 0f
                                status = getString(R.string.update_downloading)
                                lifecycleScope.launch {
                                    val outcome = withContext(Dispatchers.IO) {
                                        runCatching {
                                            Updater.download(context, available.apkUrl) { done ->
                                                progress = done
                                            }
                                        }
                                    }
                                    busy = false
                                    progress = -1f
                                    outcome.fold(
                                        onSuccess = { apk ->
                                            status = getString(R.string.update_installing)
                                            // The system installer asks for
                                            // confirmation; this app never
                                            // installs anything silently.
                                            startActivity(Updater.installIntent(context, apk))
                                        },
                                        onFailure = { error ->
                                            status = getString(
                                                R.string.update_failed,
                                                error.message ?: "",
                                            )
                                        },
                                    )
                                }
                            },
                        ) { Text(stringResource(R.string.action_install_update)) }
                    }
                }
                if (progress >= 0f) {
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    )
                }
                if (status.isNotEmpty()) {
                    Text(
                        status,
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 6.dp),
                    )
                }
            }
        }
    }

    private fun hasMic() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED

    /** Whether the notification listener is enabled for this app -- the
     *  system keeps it in a colon-separated setting, and there is no API
     *  that answers it directly. */
    private fun hasNotificationAccess(): Boolean {
        val enabled = AndroidSettings.Secure.getString(
            contentResolver,
            "enabled_notification_listeners",
        ) ?: return false
        return enabled.split(':').any { it.contains(packageName) }
    }

    private fun openNotificationAccess() {
        startActivity(Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS"))
    }

    private fun open(uri: Uri) {
        startActivity(
            Intent(Intent.ACTION_VIEW)
                .setData(uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        )
    }
}
