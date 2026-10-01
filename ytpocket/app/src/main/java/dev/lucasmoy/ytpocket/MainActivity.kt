package dev.lucasmoy.ytpocket

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.snapshotFlow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.activity.compose.BackHandler
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import coil.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The whole app: a search box, results, and two buttons per result.
 *
 * Deliberately one screen. The thing being built is "find a video, get the
 * file, correctly named" -- a navigation graph, a library browser and a
 * settings hierarchy would all be scaffolding around that one job. The only
 * other thing on screen is the update section, because a sideloaded app has
 * no store to do it for you.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { Screen() }
    }

    @Composable
    private fun Screen() {
        val context = LocalContext.current
        var query by remember { mutableStateOf("") }
        var results by remember { mutableStateOf<List<Native.Hit>>(emptyList()) }
        var searching by remember { mutableStateOf(false) }
        var message by remember { mutableStateOf("") }
        var report by remember { mutableStateOf("") }
        // Paging state. `exhausted` matters: YouTube stops offering a
        // continuation eventually, and without it the list would ask for the
        // next page on every scroll for the rest of the session.
        var loadingMore by remember { mutableStateOf(false) }
        var exhausted by remember { mutableStateOf(false) }
        var searched by remember { mutableStateOf("") }
        var showTools by remember { mutableStateOf(false) }
        val listState = androidx.compose.foundation.lazy.rememberLazyListState()
        val snackbar = remember { SnackbarHostState() }
        val scope = rememberCoroutineScope()
        val progress by DownloadService.current.collectAsState()
        val lastResult by DownloadService.last.collectAsState()

        // Notifications are how a download reports itself once the app is in
        // the background, so ask once, up front, rather than at the moment
        // the first download starts.
        //
        // In a `LaunchedEffect`, *not* in `remember`: a launcher registers
        // itself as part of composition, and launching one from inside the
        // composition that creates it throws "Attempting to launch an
        // unregistered ActivityResultLauncher" -- which killed the app on
        // open, before anything was drawn. `LaunchedEffect` runs after the
        // composition has been applied, which is when the launcher exists.
        val notificationLauncher = rememberLauncherForActivityResult(
            ActivityResultContracts.RequestPermission()
        ) { }
        LaunchedEffect(Unit) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
                android.content.pm.PackageManager.PERMISSION_GRANTED
            ) {
                notificationLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }

        fun runSearch() {
            val text = query.trim()
            if (text.isEmpty()) return
            searching = true
            exhausted = false
            searched = text
            message = ""
            lifecycleScope.launch {
                val outcome = withContext(Dispatchers.IO) {
                    runCatching {
                        // A pasted link is a search for one video, not a
                        // search at all -- that is what people do with a
                        // share sheet.
                        if (text.contains("youtu", ignoreCase = true) && text.contains("/")) {
                            val resolved = Native.resolveVideo(text)
                            listOf(
                                Native.Hit(
                                    id = resolved.id,
                                    title = resolved.title,
                                    channel = resolved.channel,
                                    duration = resolved.duration,
                                    views = null,
                                    published = null,
                                    thumbnail = "https://i.ytimg.com/vi/${resolved.id}/hqdefault.jpg",
                                )
                            )
                        } else {
                            Native.searchVideos(text)
                        }
                    }
                }
                searching = false
                outcome.fold(
                    onSuccess = { hits ->
                        results = hits
                        if (hits.isEmpty()) message = getString(R.string.no_results)
                    },
                    onFailure = { message = getString(R.string.search_failed, it.message ?: "") },
                )
            }
        }

        fun loadMore() {
            if (loadingMore || exhausted || searching) return
            loadingMore = true
            lifecycleScope.launch {
                val outcome = withContext(Dispatchers.IO) {
                    runCatching { Native.moreVideos(searched) }
                }
                loadingMore = false
                outcome.fold(
                    onSuccess = { more ->
                        // Empty means YouTube has no continuation left, not
                        // that something went wrong.
                        if (more.isEmpty()) exhausted = true
                        else results = results + more.filter { fresh ->
                            // YouTube does repeat itself across pages
                            // occasionally, and a duplicate key crashes a
                            // LazyColumn.
                            results.none { it.id == fresh.id }
                        }
                    },
                    onFailure = {
                        exhausted = true
                        message = getString(R.string.search_failed, it.message ?: "")
                    },
                )
            }
        }

        // Ask for the next page shortly before the bottom, so scrolling never
        // stops on an empty screen.
        LaunchedEffect(listState, searched) {
            snapshotFlow {
                listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1
            }.collect { lastVisible ->
                if (results.isNotEmpty() && lastVisible >= results.size - 4) loadMore()
            }
        }

        AppTheme {
            Surface(modifier = Modifier.fillMaxSize()) {
              Box(modifier = Modifier.fillMaxSize()) {
                Column(modifier = Modifier.padding(horizontal = 14.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(top = 16.dp, bottom = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            stringResource(R.string.app_name),
                            fontSize = 22.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.weight(1f),
                        )
                        // Everything that is not "find a video and get the
                        // file" lives behind this: the results list now pages
                        // forever, so the bottom of the screen belongs to
                        // results and nothing else.
                        IconButton(onClick = { showTools = true }) {
                            Icon(
                                Icons.Filled.Settings,
                                contentDescription = stringResource(R.string.action_tools),
                            )
                        }
                    }

                    OutlinedTextField(
                        value = query,
                        onValueChange = { query = it },
                        label = { Text(stringResource(R.string.search_label)) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                            imeAction = ImeAction.Search
                        ),
                        keyboardActions = androidx.compose.foundation.text.KeyboardActions(
                            onSearch = { runSearch() }
                        ),
                        trailingIcon = {
                            TextButton(onClick = { runSearch() }, enabled = !searching) {
                                Text(stringResource(R.string.action_search))
                            }
                        },
                    )

                    if (searching) {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            CircularProgressIndicator(modifier = Modifier.size(18.dp))
                            Text(
                                stringResource(R.string.searching),
                                fontSize = 13.sp,
                                modifier = Modifier.padding(start = 10.dp),
                            )
                        }
                    }

                    if (message.isNotEmpty()) {
                        Text(
                            message,
                            fontSize = 13.sp,
                            color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.padding(vertical = 8.dp),
                        )
                    }

                    // What is downloading right now, and what just finished.
                    progress?.let { active ->
                        Card(modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                            Column(Modifier.padding(12.dp)) {
                                Text(active.title, fontSize = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                Text(
                                    active.step,
                                    fontSize = 12.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                LinearProgressIndicator(
                                    progress = { active.fraction },
                                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                                )
                            }
                        }
                    }
                    if (progress == null) {
                        lastResult?.let { result ->
                            Card(modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                                Row(
                                    Modifier.padding(12.dp).fillMaxWidth(),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Column(Modifier.weight(1f)) {
                                        Text(
                                            when {
                                                result.organised -> stringResource(R.string.organise_done)
                                                result.error == null -> stringResource(R.string.download_done)
                                                else -> stringResource(R.string.download_failed)
                                            },
                                            fontSize = 13.sp,
                                            fontWeight = FontWeight.SemiBold,
                                        )
                                        Text(
                                            result.error ?: result.name,
                                            fontSize = 12.sp,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    if (result.uri != null) {
                                        OutlinedButton(onClick = {
                                            startActivity(
                                                Intent(Intent.ACTION_VIEW).apply {
                                                    setDataAndType(
                                                        result.uri,
                                                        // From the result, not the name: the
                                                        // name carries details after the
                                                        // extension, so `endsWith(".mp3")`
                                                        // opened every MP3 as a video.
                                                        if (result.audio) "audio/mpeg" else "video/mp4",
                                                    )
                                                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                                }
                                            )
                                        }) { Text(stringResource(R.string.action_open)) }
                                    }
                                }
                            }
                        }
                    }

                    LazyColumn(
                        state = listState,
                        modifier = Modifier.fillMaxWidth().weight(1f),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(results, key = { it.id }) { hit ->
                            ResultRow(hit) { kind ->
                                val ahead = DownloadService.start(context, hit.id, hit.title, kind)
                                val what = getString(if (kind == DownloadService.KIND_MP3) R.string.action_mp3 else R.string.action_mp4)
                                val text = if (ahead == 0) getString(R.string.queued_now, what, hit.title)
                                else getString(R.string.queued_after, what, hit.title, ahead)
                                scope.launch {
                                    // A new tap replaces the old message rather
                                    // than queueing behind it.
                                    snackbar.currentSnackbarData?.dismiss()
                                    snackbar.showSnackbar(text, withDismissAction = true, duration = SnackbarDuration.Short)
                                }
                            }
                        }
                        if (loadingMore) {
                            item {
                                Row(
                                    modifier = Modifier.fillMaxWidth().padding(vertical = 14.dp),
                                    horizontalArrangement = Arrangement.Center,
                                ) { CircularProgressIndicator(modifier = Modifier.size(20.dp)) }
                            }
                        }
                        if (exhausted && results.isNotEmpty()) {
                            item {
                                Text(
                                    stringResource(R.string.no_more_results),
                                    fontSize = 12.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.fillMaxWidth().padding(vertical = 14.dp),
                                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                )
                            }
                        }
                    }
                }

                SnackbarHost(
                    hostState = snackbar,
                    modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                )

                if (showTools) {
                    ToolsSheet(
                        report = report,
                        onReport = { report = it },
                        target = { query.trim().ifEmpty { results.firstOrNull()?.id.orEmpty() } },
                        onClose = { showTools = false },
                    )
                }
              }
            }
        }
    }

    /**
     * Diagnostics and updates, on their own screen.
     *
     * They used to sit under the results list, which stopped working the
     * moment the list started paging: there is no "under" a list that keeps
     * growing.
     */
    @Composable
    private fun ToolsSheet(
        report: String,
        onReport: (String) -> Unit,
        target: () -> String,
        onClose: () -> Unit,
    ) {
        // Back closes the settings, as on every other Android screen, instead
        // of leaving the app.
        BackHandler(onBack = onClose)
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 14.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = 16.dp, bottom = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        stringResource(R.string.action_tools),
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = onClose) { Text(stringResource(R.string.action_close)) }
                }
                // On screen on purpose: three bug reports in a row were about
                // versions that had already been fixed, and there was no way
                // to tell from the app which one was running.
                Text(
                    stringResource(R.string.update_current, BuildConfig.VERSION_NAME),
                    fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(bottom = 10.dp),
                )
                FoldersSection()
                MusicSection()
                HorizontalDivider(Modifier.padding(vertical = 10.dp))
                // Diagnostics: which YouTube client works from *this*
                // network. A 403 depends on where the phone is, so this is
                // the only way to tell what is actually happening on a
                // device the author cannot reach.
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    var running by remember { mutableStateOf(false) }
                    TextButton(
                        enabled = !running,
                        onClick = {
                            val target = target()
                            if (target.isEmpty()) {
                                onReport(getString(R.string.diag_need_target))
                                return@TextButton
                            }
                            onReport(getString(R.string.diag_running))
                            running = true
                            lifecycleScope.launch {
                                val outcome = withContext(Dispatchers.IO) {
                                    runCatching { Native.diagnostics(target) }
                                }
                                running = false
                                onReport(outcome.getOrElse { "error: ${it.message}" })
                            }
                        },
                    ) { Text(stringResource(R.string.action_diagnose)) }
                    if (report.isNotEmpty()) {
                        TextButton(onClick = {
                            startActivity(
                                Intent.createChooser(
                                    Intent(Intent.ACTION_SEND).apply {
                                        type = "text/plain"
                                        putExtra(Intent.EXTRA_TEXT, report)
                                    },
                                    getString(R.string.action_share_report),
                                )
                            )
                        }) { Text(stringResource(R.string.action_share_report)) }
                    }
                }
                if (report.isNotEmpty()) {
                    androidx.compose.foundation.text.selection.SelectionContainer {
                        Text(
                            report,
                            fontSize = 11.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(bottom = 6.dp),
                        )
                    }
                }
                UpdateSection()
            }
        }
    }

    /**
     * Where MP3s and MP4s go, separately. The picker is the system's own
     * folder chooser (Storage Access Framework), which is the only way to
     * reach any folder -- an SD card included -- without "all files" access.
     */
    @Composable
    private fun FoldersSection() {
        val context = LocalContext.current
        var mp3Label by remember { mutableStateOf(Settings.label(context, audio = true)) }
        var mp4Label by remember { mutableStateOf(Settings.label(context, audio = false)) }
        var error by remember { mutableStateOf("") }
        var picking by remember { mutableStateOf(true) }
        val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
            if (uri == null) return@rememberLauncherForActivityResult
            runCatching { Settings.setTree(context, picking, uri) }
                .onSuccess { error = "" }
                .onFailure { error = getString(R.string.folder_failed, it.message ?: "") }
            mp3Label = Settings.label(context, audio = true)
            mp4Label = Settings.label(context, audio = false)
        }

        Card(modifier = Modifier.fillMaxWidth().padding(bottom = 10.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(stringResource(R.string.section_folders), fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                for (audio in listOf(true, false)) {
                    val label = if (audio) mp3Label else mp4Label
                    Text(
                        stringResource(if (audio) R.string.folder_mp3 else R.string.folder_mp4),
                        fontSize = 13.sp,
                        fontWeight = FontWeight.Medium,
                        modifier = Modifier.padding(top = 10.dp),
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            label,
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f),
                        )
                        if (Settings.tree(context, audio) != null) {
                            TextButton(onClick = {
                                Settings.setTree(context, audio, null)
                                mp3Label = Settings.label(context, audio = true)
                                mp4Label = Settings.label(context, audio = false)
                            }) { Text(stringResource(R.string.action_default_folder)) }
                        }
                        OutlinedButton(onClick = {
                            picking = audio
                            picker.launch(Settings.tree(context, audio))
                        }) { Text(stringResource(R.string.action_change_folder)) }
                    }
                }
                if (error.isNotEmpty()) {
                    Text(error, fontSize = 12.sp, color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }

    /** Tagging and filing MP3s: the switch for new ones, and the button for the rest. */
    @Composable
    private fun MusicSection() {
        val context = LocalContext.current
        var auto by remember { mutableStateOf(Settings.autoOrganise(context)) }
        var status by remember { mutableStateOf("") }
        Card(modifier = Modifier.fillMaxWidth().padding(bottom = 10.dp)) {
            Column(Modifier.padding(12.dp)) {
                Text(stringResource(R.string.section_music), fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                Row(
                    modifier = Modifier.fillMaxWidth().padding(top = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.auto_organise), fontSize = 13.sp, fontWeight = FontWeight.Medium)
                        Text(
                            stringResource(R.string.auto_organise_hint),
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Switch(checked = auto, onCheckedChange = {
                        auto = it
                        Settings.setAutoOrganise(context, it)
                    })
                }
                Text(
                    stringResource(R.string.organise_hint),
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 10.dp),
                )
                Button(
                    modifier = Modifier.padding(top = 8.dp),
                    onClick = {
                        DownloadService.organise(context)
                        status = getString(R.string.organise_queued)
                    },
                ) { Text(stringResource(R.string.action_organise)) }
                if (status.isNotEmpty()) {
                    Text(status, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
                }
            }
        }
    }

    /** The same in-app update flow the sibling apps have. */
    @Composable
    private fun UpdateSection() {
        val context = LocalContext.current
        var update by remember { mutableStateOf<Updater.Available?>(null) }
        var busy by remember { mutableStateOf(false) }
        var downloadProgress by remember { mutableFloatStateOf(-1f) }
        var status by remember { mutableStateOf("") }

        Card(modifier = Modifier.fillMaxWidth().padding(top = 14.dp, bottom = 20.dp)) {
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
                    modifier = Modifier.padding(top = 8.dp),
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
                                            available.hasUpdate -> getString(R.string.update_no_apk, available.latest)
                                            else -> getString(R.string.update_none, available.current)
                                        }
                                    },
                                    onFailure = { status = getString(R.string.update_failed, it.message ?: "") },
                                )
                            }
                        },
                    ) { Text(stringResource(R.string.action_check_updates)) }

                    val available = update
                    if (available != null && available.hasUpdate && available.apkUrl.isNotEmpty()) {
                        Button(
                            enabled = !busy,
                            onClick = {
                                // Permission first: downloading 20MB and then
                                // discovering the install is blocked is the
                                // worst possible order.
                                if (!Updater.canInstall(context)) {
                                    status = getString(R.string.update_need_permission)
                                    startActivity(Updater.installPermissionIntent(context))
                                    return@Button
                                }
                                busy = true
                                downloadProgress = 0f
                                lifecycleScope.launch {
                                    val apk = withContext(Dispatchers.IO) {
                                        runCatching {
                                            Updater.download(context, available.apkUrl) { downloadProgress = it }
                                        }
                                    }
                                    busy = false
                                    downloadProgress = -1f
                                    apk.fold(
                                        onSuccess = {
                                            status = getString(R.string.update_confirm)
                                            startActivity(Updater.installIntent(context, it))
                                        },
                                        onFailure = { status = getString(R.string.update_failed, it.message ?: "") },
                                    )
                                }
                            },
                        ) {
                            Text(
                                if (downloadProgress >= 0f) {
                                    stringResource(R.string.action_downloading, (downloadProgress * 100).toInt())
                                } else {
                                    stringResource(R.string.action_install_update, available.latest)
                                }
                            )
                        }
                    }
                    OutlinedButton(onClick = {
                        startActivity(
                            Intent(Intent.ACTION_VIEW, Uri.parse(update?.pageUrl ?: Updater.releasesPage))
                        )
                    }) { Text(stringResource(R.string.action_releases)) }
                }
                if (status.isNotEmpty()) {
                    Text(status, fontSize = 13.sp, modifier = Modifier.padding(top = 8.dp))
                }
            }
        }
    }

    @Composable
    private fun stringResource(id: Int, vararg args: Any): String =
        if (args.isEmpty()) LocalContext.current.getString(id)
        else LocalContext.current.getString(id, *args)
}
