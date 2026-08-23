package dev.lucasmoy.livesubs

import android.Manifest
import android.content.Intent
import android.graphics.Color as AndroidColor
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings as AndroidSettings
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The settings screen -- the only window this app has.
 *
 * Everything it can offer mirrors the desktop app's settings window, in the
 * same order, minus what a phone cannot do and plus what a phone demands:
 * three permissions the user has to grant by hand (microphone, drawing over
 * other apps, and screen-capture consent for playback audio), each with the
 * reason next to the button.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { SettingsScreen() }
    }

    @Composable
    private fun SettingsScreen() {
        val context = LocalContext.current
        val prefs = remember { Prefs.get(context) }
        val settings by prefs.settings.collectAsState()
        val running by CaptionService.isRunning.collectAsState()
        val status by CaptionService.statusText.collectAsState()

        var modelProgress by remember { mutableFloatStateOf(-1f) }
        var message by remember { mutableStateOf("") }
        var update by remember { mutableStateOf<Updater.Available?>(null) }
        var updateBusy by remember { mutableStateOf(false) }
        var updateProgress by remember { mutableFloatStateOf(-1f) }
        var updateMessage by remember { mutableStateOf("") }
        var overlayGranted by remember { mutableStateOf(AndroidSettings.canDrawOverlays(context)) }
        var micGranted by remember {
            mutableStateOf(
                checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
                    android.content.pm.PackageManager.PERMISSION_GRANTED
            )
        }

        val permissionLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.RequestMultiplePermissions()
        ) { granted ->
            micGranted = granted[Manifest.permission.RECORD_AUDIO] ?: micGranted
        }
        val overlayLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) { overlayGranted = AndroidSettings.canDrawOverlays(context) }
        // Screen-capture consent is what unlocks "system audio": the same
        // dialog a screen recorder shows, because it is the same permission.
        val projectionLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) { result ->
            if (result.resultCode == RESULT_OK && result.data != null) {
                CaptionService.start(context, result.data)
            } else {
                message = getString(R.string.msg_projection_denied)
                CaptionService.start(context)
            }
        }
        val transcriptLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("text/plain")
        ) { uri: Uri? ->
            if (uri != null) {
                contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_READ_URI_PERMISSION,
                )
                prefs.update { it.copy(logUri = uri.toString(), logEnabled = true) }
            }
        }

        fun startCapture() {
            val missing = buildList {
                if (!micGranted) add(Manifest.permission.RECORD_AUDIO)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    add(Manifest.permission.POST_NOTIFICATIONS)
                }
            }
            if (missing.isNotEmpty()) {
                permissionLauncher.launch(missing.toTypedArray())
                return
            }
            if (!overlayGranted) {
                message = getString(R.string.msg_need_overlay)
                overlayLauncher.launch(
                    Intent(
                        AndroidSettings.ACTION_MANAGE_OVERLAY_PERMISSION,
                        Uri.parse("package:$packageName"),
                    )
                )
                return
            }
            if (!ModelStore.isDownloaded(context, settings.model)) {
                message = getString(R.string.msg_need_model)
                return
            }
            if (settings.captureSystem) {
                val manager = getSystemService(MediaProjectionManager::class.java)
                projectionLauncher.launch(manager.createScreenCaptureIntent())
            } else {
                CaptionService.start(context)
            }
        }

        MaterialTheme {
            Surface(modifier = Modifier.fillMaxSize()) {
                Column(
                    modifier = Modifier
                        .verticalScroll(rememberScrollState())
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Text(
                        stringResource(R.string.app_name),
                        fontSize = 24.sp,
                        fontWeight = FontWeight.Bold,
                    )

                    // ---- estado ----
                    Section(stringResource(R.string.section_status)) {
                        StatusLine(running, if (running) status.ifEmpty { stringResource(R.string.status_listening) } else stringResource(R.string.status_stopped))
                        StatusLine(micGranted, stringResource(R.string.status_mic_permission))
                        StatusLine(overlayGranted, stringResource(R.string.status_overlay_permission))
                        StatusLine(
                            ModelStore.isDownloaded(context, settings.model),
                            stringResource(R.string.status_model, settings.model),
                        )
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            modifier = Modifier.padding(top = 6.dp),
                        ) {
                            Button(onClick = {
                                if (running) CaptionService.stop(context) else startCapture()
                            }) {
                                Text(stringResource(if (running) R.string.action_stop else R.string.action_start))
                            }
                            OutlinedButton(
                                enabled = running,
                                onClick = { prefs.update { it.copy(paused = !it.paused) } },
                            ) {
                                Text(stringResource(if (settings.paused) R.string.action_resume else R.string.action_pause))
                            }
                            OutlinedButton(onClick = {
                                if (!overlayGranted) {
                                    overlayLauncher.launch(
                                        Intent(
                                            AndroidSettings.ACTION_MANAGE_OVERLAY_PERMISSION,
                                            Uri.parse("package:$packageName"),
                                        )
                                    )
                                } else {
                                    CaptionService.preview(context)
                                }
                            }) { Text(stringResource(R.string.action_preview)) }
                        }
                        if (message.isNotEmpty()) {
                            Text(message, fontSize = 13.sp, color = MaterialTheme.colorScheme.error)
                        }
                    }

                    // ---- audio ----
                    Section(stringResource(R.string.section_audio)) {
                        SwitchRow(
                            stringResource(R.string.setting_mic),
                            stringResource(R.string.setting_mic_hint),
                            settings.captureMic,
                        ) { value -> prefs.update { it.copy(captureMic = value) } }
                        SwitchRow(
                            stringResource(R.string.setting_system),
                            stringResource(R.string.setting_system_hint),
                            settings.captureSystem,
                        ) { value -> prefs.update { it.copy(captureSystem = value) } }
                        SliderRow(
                            label = stringResource(R.string.setting_sensitivity),
                            hint = stringResource(R.string.setting_sensitivity_hint),
                            value = settings.sensitivity,
                            range = 0.3f..3f,
                            steps = 26,
                            format = { "%.1f×".format(it) },
                        ) { value -> prefs.update { it.copy(sensitivity = value) } }
                    }

                    // ---- reconocimiento ----
                    Section(stringResource(R.string.section_recognition)) {
                        DropdownRow(
                            label = stringResource(R.string.setting_model),
                            hint = stringResource(R.string.setting_model_hint),
                            options = ModelStore.MODELS.map { (name, size) ->
                                name to "$name ($size)${if (ModelStore.isDownloaded(context, name)) " ✓" else ""}"
                            },
                            selected = settings.model,
                        ) { value -> prefs.update { it.copy(model = value) } }
                        if (!ModelStore.isDownloaded(context, settings.model)) {
                            Button(
                                enabled = modelProgress < 0f,
                                onClick = {
                                    val model = settings.model
                                    modelProgress = 0f
                                    lifecycleScope.launch {
                                        val outcome = withContext(Dispatchers.IO) {
                                            runCatching { ModelStore.download(context, model) { modelProgress = it } }
                                        }
                                        modelProgress = -1f
                                        message = outcome.fold(
                                            onSuccess = { getString(R.string.msg_model_ready, model) },
                                            onFailure = { getString(R.string.msg_model_failed, it.message ?: "") },
                                        )
                                    }
                                },
                            ) {
                                Text(
                                    if (modelProgress >= 0f) {
                                        stringResource(R.string.action_downloading, (modelProgress * 100).toInt())
                                    } else {
                                        stringResource(R.string.action_download_model)
                                    }
                                )
                            }
                        }
                        DropdownRow(
                            label = stringResource(R.string.setting_spoken_language),
                            hint = stringResource(R.string.setting_spoken_language_hint),
                            options = listOf(
                                "auto" to stringResource(R.string.language_auto),
                                "en" to stringResource(R.string.language_en),
                                "es" to stringResource(R.string.language_es),
                                "fr" to stringResource(R.string.language_fr),
                            ),
                            selected = settings.sourceLanguage,
                        ) { value -> prefs.update { it.copy(sourceLanguage = value) } }
                    }

                    // ---- traducción ----
                    Section(stringResource(R.string.section_translation)) {
                        DropdownRow(
                            label = stringResource(R.string.setting_target_language),
                            hint = stringResource(R.string.setting_target_language_hint),
                            options = listOf(
                                "off" to stringResource(R.string.language_off),
                                "en" to stringResource(R.string.language_en),
                                "es" to stringResource(R.string.language_es),
                                "fr" to stringResource(R.string.language_fr),
                            ),
                            selected = settings.targetLanguage,
                        ) { value -> prefs.update { it.copy(targetLanguage = value) } }
                        SwitchRow(
                            stringResource(R.string.setting_show_original),
                            stringResource(R.string.setting_show_original_hint),
                            settings.showOriginal,
                        ) { value -> prefs.update { it.copy(showOriginal = value) } }
                        Text(
                            stringResource(R.string.translation_note),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    // ---- aspecto ----
                    Section(stringResource(R.string.section_appearance)) {
                        DropdownRow(
                            label = stringResource(R.string.setting_position),
                            hint = null,
                            options = listOf(
                                "bottom" to stringResource(R.string.position_bottom),
                                "center" to stringResource(R.string.position_center),
                                "top" to stringResource(R.string.position_top),
                            ),
                            selected = settings.anchor,
                        ) { value -> prefs.update { it.copy(anchor = value) } }
                        SliderRow(
                            stringResource(R.string.setting_margin),
                            stringResource(R.string.setting_margin_hint),
                            settings.margin.toFloat(),
                            0f..400f,
                            39,
                            { "${it.toInt()} dp" },
                        ) { value -> prefs.update { it.copy(margin = value.toInt()) } }
                        SliderRow(
                            stringResource(R.string.setting_width),
                            null,
                            settings.widthPercent.toFloat(),
                            40f..100f,
                            11,
                            { "${it.toInt()}%" },
                        ) { value -> prefs.update { it.copy(widthPercent = value.toInt()) } }
                        SliderRow(
                            stringResource(R.string.setting_font_size),
                            null,
                            settings.fontSize.toFloat(),
                            12f..40f,
                            27,
                            { "${it.toInt()} sp" },
                        ) { value -> prefs.update { it.copy(fontSize = value.toInt()) } }
                        SliderRow(
                            stringResource(R.string.setting_max_lines),
                            stringResource(R.string.setting_max_lines_hint),
                            settings.maxLines.toFloat(),
                            1f..4f,
                            2,
                            { "${it.toInt()}" },
                        ) { value -> prefs.update { it.copy(maxLines = value.toInt()) } }
                        SliderRow(
                            stringResource(R.string.setting_hide_after),
                            stringResource(R.string.setting_hide_after_hint),
                            settings.hideAfterMs / 1000f,
                            1f..20f,
                            19,
                            { "%.0f s".format(it) },
                        ) { value -> prefs.update { it.copy(hideAfterMs = (value * 1000).toInt()) } }
                        SliderRow(
                            stringResource(R.string.setting_bg_opacity),
                            stringResource(R.string.setting_bg_opacity_hint),
                            settings.backgroundOpacity,
                            0f..1f,
                            20,
                            { "${(it * 100).toInt()}%" },
                        ) { value -> prefs.update { it.copy(backgroundOpacity = value) } }
                        ColorRow(
                            stringResource(R.string.setting_mic_color),
                            settings.micColor,
                            VOICE_COLORS,
                        ) { value -> prefs.update { it.copy(micColor = value) } }
                        ColorRow(
                            stringResource(R.string.setting_system_color),
                            settings.systemColor,
                            VOICE_COLORS,
                        ) { value -> prefs.update { it.copy(systemColor = value) } }
                        ColorRow(
                            stringResource(R.string.setting_bg_color),
                            settings.backgroundColor,
                            PLATE_COLORS,
                        ) { value -> prefs.update { it.copy(backgroundColor = value) } }
                    }

                    // ---- transcripción ----
                    Section(stringResource(R.string.section_transcript)) {
                        SwitchRow(
                            stringResource(R.string.setting_log),
                            stringResource(R.string.setting_log_hint),
                            settings.logEnabled,
                        ) { value -> prefs.update { it.copy(logEnabled = value) } }
                        Text(
                            settings.logUri?.let { Uri.decode(it).substringAfterLast('/') }
                                ?: stringResource(R.string.transcript_none),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        OutlinedButton(onClick = { transcriptLauncher.launch("livesubs-transcript.txt") }) {
                            Text(stringResource(R.string.action_pick_transcript))
                        }
                    }

                    // ---- actualizaciones ----
                    Section(stringResource(R.string.section_updates)) {
                        Text(
                            stringResource(R.string.update_current, BuildConfig.VERSION_NAME),
                            fontSize = 13.sp,
                        )
                        Text(
                            stringResource(R.string.update_hint),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutlinedButton(
                                enabled = !updateBusy,
                                onClick = {
                                    updateBusy = true
                                    updateMessage = getString(R.string.update_checking)
                                    lifecycleScope.launch {
                                        val result = withContext(Dispatchers.IO) {
                                            Updater.check(BuildConfig.VERSION_NAME)
                                        }
                                        updateBusy = false
                                        result.fold(
                                            onSuccess = { available ->
                                                update = available
                                                updateMessage = when {
                                                    available.hasUpdate && available.apkUrl.isNotEmpty() ->
                                                        getString(R.string.update_available, available.latest)
                                                    available.hasUpdate ->
                                                        getString(R.string.update_no_apk, available.latest)
                                                    else -> getString(R.string.update_none, available.current)
                                                }
                                            },
                                            onFailure = {
                                                updateMessage = getString(R.string.update_failed, it.message ?: "")
                                            },
                                        )
                                    }
                                },
                            ) { Text(stringResource(R.string.action_check_updates)) }

                            val available = update
                            if (available != null && available.hasUpdate && available.apkUrl.isNotEmpty()) {
                                Button(
                                    enabled = !updateBusy,
                                    onClick = {
                                        // The permission check comes first: downloading
                                        // 20MB and *then* discovering the install is
                                        // blocked is the worst order to do this in.
                                        if (!Updater.canInstall(context)) {
                                            updateMessage = getString(R.string.update_need_permission)
                                            startActivity(Updater.installPermissionIntent(context))
                                            return@Button
                                        }
                                        updateBusy = true
                                        updateProgress = 0f
                                        lifecycleScope.launch {
                                            val apk = withContext(Dispatchers.IO) {
                                                runCatching {
                                                    Updater.download(context, available.apkUrl) { updateProgress = it }
                                                }
                                            }
                                            updateBusy = false
                                            updateProgress = -1f
                                            apk.fold(
                                                onSuccess = {
                                                    updateMessage = getString(R.string.update_confirm)
                                                    startActivity(Updater.installIntent(context, it))
                                                },
                                                onFailure = {
                                                    updateMessage = getString(R.string.update_failed, it.message ?: "")
                                                },
                                            )
                                        }
                                    },
                                ) {
                                    Text(
                                        if (updateProgress >= 0f) {
                                            stringResource(R.string.action_downloading, (updateProgress * 100).toInt())
                                        } else {
                                            stringResource(R.string.action_install_update, available.latest)
                                        }
                                    )
                                }
                            }
                            OutlinedButton(onClick = {
                                startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(update?.pageUrl ?: Updater.releasesPage)))
                            }) { Text(stringResource(R.string.action_open_releases)) }
                        }
                        if (updateMessage.isNotEmpty()) {
                            Text(updateMessage, fontSize = 13.sp)
                        }
                        update?.notes?.takeIf { it.isNotBlank() }?.let { notes ->
                            Text(
                                notes,
                                fontSize = 12.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }

                    Text(
                        stringResource(R.string.calls_note),
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(1.dp))
                }
            }
        }
    }

    // ---- small building blocks ---------------------------------------

    @Composable
    private fun Section(title: String, content: @Composable () -> Unit) {
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(title.uppercase(), fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                content()
            }
        }
    }

    @Composable
    private fun StatusLine(ok: Boolean, text: String) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Spacer(
                Modifier
                    .size(9.dp)
                    .background(if (ok) Color(0xFF1A8A4A) else Color(0xFFD33A2C), CircleShape)
            )
            Text(text, fontSize = 13.sp, modifier = Modifier.padding(start = 8.dp))
        }
    }

    @Composable
    private fun SwitchRow(label: String, hint: String?, value: Boolean, onChange: (Boolean) -> Unit) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(label, fontSize = 14.sp)
                if (hint != null) {
                    Text(hint, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Switch(checked = value, onCheckedChange = onChange)
        }
    }

    @Composable
    private fun SliderRow(
        label: String,
        hint: String?,
        value: Float,
        range: ClosedFloatingPointRange<Float>,
        steps: Int,
        format: (Float) -> String,
        onChange: (Float) -> Unit,
    ) {
        Column {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(label, fontSize = 14.sp, modifier = Modifier.weight(1f))
                Text(format(value), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (hint != null) {
                Text(hint, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Slider(value = value, onValueChange = onChange, valueRange = range, steps = steps)
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun DropdownRow(
        label: String,
        hint: String?,
        options: List<Pair<String, String>>,
        selected: String,
        onChange: (String) -> Unit,
    ) {
        var expanded by remember { mutableStateOf(false) }
        Column {
            Text(label, fontSize = 14.sp)
            if (hint != null) {
                Text(hint, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            ExposedDropdownMenuBox(expanded = expanded, onExpandedChange = { expanded = it }) {
                TextField(
                    value = options.firstOrNull { it.first == selected }?.second ?: selected,
                    onValueChange = {},
                    readOnly = true,
                    trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .menuAnchor(),
                )
                ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                    options.forEach { (value, text) ->
                        DropdownMenuItem(
                            text = { Text(text) },
                            onClick = {
                                expanded = false
                                onChange(value)
                            },
                        )
                    }
                }
            }
        }
    }

    /**
     * A row of swatches instead of a colour picker: the thing that matters
     * is that the two voices are clearly different and both legible on a
     * dark plate, and four good choices get there faster than a hue wheel.
     */
    @Composable
    private fun ColorRow(label: String, selected: Int, palette: List<Int>, onChange: (Int) -> Unit) {
        Column {
            Text(label, fontSize = 14.sp)
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp), modifier = Modifier.padding(top = 6.dp)) {
                palette.forEach { color ->
                    val isSelected = color == selected
                    OutlinedButton(
                        onClick = { onChange(color) },
                        modifier = Modifier.size(width = 52.dp, height = 34.dp),
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(0.dp),
                    ) {
                        Spacer(
                            Modifier
                                .size(if (isSelected) 22.dp else 16.dp)
                                .background(Color(color), CircleShape)
                        )
                    }
                }
            }
        }
    }

    @Composable
    private fun stringResource(id: Int, vararg args: Any): String =
        if (args.isEmpty()) LocalContext.current.getString(id)
        else LocalContext.current.getString(id, *args)

    private companion object {
        val VOICE_COLORS = listOf(
            0xFF7AD7FF.toInt(), // cyan: the desktop app's microphone colour
            AndroidColor.WHITE,
            0xFFFFD166.toInt(),
            0xFF9BE564.toInt(),
        )
        val PLATE_COLORS = listOf(
            AndroidColor.BLACK,
            0xFF1B1B1E.toInt(),
            0xFF102030.toInt(),
            0xFF2B0F1A.toInt(),
        )
    }
}
