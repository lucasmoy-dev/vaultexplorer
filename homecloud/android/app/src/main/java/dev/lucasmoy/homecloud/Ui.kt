package dev.lucasmoy.homecloud

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import android.os.Environment
import androidx.compose.foundation.Image
import androidx.compose.material.icons.Icons
import android.content.Intent
import android.provider.DocumentsContract
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CleaningServices
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.LinkOff
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** Fast enough that a sync looks live, slow enough not to hammer the engine. */
private const val POLL_MS = 1500L

@Composable
fun StoragePermissionScreen(onGrant: () -> Unit) {
    Surface(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().padding(28.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("HomeCloud necesita ver tus carpetas", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(10.dp))
            Text(
                "Para sincronizar una carpeta hay que poder leerla y escribirla. Android solo " +
                    "concede ese permiso desde sus ajustes.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(20.dp))
            Button(onClick = onGrant) { Text("Abrir ajustes de Android") }
        }
    }
}

@Composable
fun HomeScreen() {
    val scope = rememberCoroutineScope()
    var folders by remember { mutableStateOf<List<SharedFolder>>(emptyList()) }
    var invitations by remember { mutableStateOf<List<Invitation>>(emptyList()) }
    var ready by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    var showSettings by remember { mutableStateOf(false) }
    var showJoin by remember { mutableStateOf(false) }
    var pickForShare by remember { mutableStateOf(false) }
    var codeOnScreen by remember { mutableStateOf<Pair<String, String>?>(null) }
    var openFolder by remember { mutableStateOf<SharedFolder?>(null) }

    suspend fun refresh() = withContext(Dispatchers.IO) {
        runCatching {
            val f = Repo.folders()
            val i = Repo.invitations()
            withContext(Dispatchers.Main) {
                folders = f
                invitations = i
                ready = true
                openFolder = openFolder?.let { open -> f.find { it.id == open.id } }
            }
        }.onFailure {
            // Before the engine answers, failures are just "not up yet".
            if (ready) withContext(Dispatchers.Main) { error = it.message }
        }
    }

    LaunchedEffect(Unit) {
        while (true) {
            refresh()
            delay(POLL_MS)
        }
    }

    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("HomeCloud", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                IconButton(onClick = { showSettings = true }) {
                    Icon(Icons.Filled.Settings, contentDescription = "Ajustes")
                }
            }
            Spacer(Modifier.height(8.dp))

            error?.let {
                Banner(tone = MaterialTheme.colorScheme.error) {
                    Text(it, Modifier.clickable { error = null })
                }
                Spacer(Modifier.height(8.dp))
            }

            folders.filter { doesNotFit(it.pendingBytes, it.freeBytes) }.forEach { folder ->
                Banner(tone = MaterialTheme.colorScheme.error) {
                    Text(
                        "A «${folder.label}» le faltan ${formatBytes(folder.pendingBytes)} por bajar " +
                            "y en ese disco quedan ${formatBytes(folder.freeBytes ?: 0)}. Libera " +
                            "${formatBytes(shortfall(folder.pendingBytes, folder.freeBytes ?: 0))} " +
                            "o guárdala en otro sitio.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Spacer(Modifier.height(8.dp))
            }

            invitations.forEach { invitation ->
                InvitationBanner(
                    invitation = invitation,
                    onDone = { scope.launch { refresh() } },
                    onError = { error = it },
                )
                Spacer(Modifier.height(8.dp))
            }

            if (!ready) {
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(12.dp))
                        Text("Arrancando…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            } else if (folders.isEmpty()) {
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text("Todavía no compartes nada", fontWeight = FontWeight.Medium)
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "Comparte una carpeta del teléfono, o únete a una que ya exista en otro dispositivo.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            } else {
                LazyColumn(Modifier.weight(1f)) {
                    items(folders) { folder ->
                        FolderRow(folder) { openFolder = folder }
                        if (folder.conflicts > 0) {
                            Text(
                                if (folder.conflicts == 1L)
                                    "1 fichero se editó en dos sitios a la vez. Se guardaron las dos versiones."
                                else
                                    "${folder.conflicts} ficheros se editaron en dos sitios a la vez. Se guardaron las dos versiones.",
                                Modifier.padding(start = 30.dp, top = 4.dp, bottom = 4.dp),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Spacer(Modifier.height(8.dp))
                    }
                }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { pickForShare = true }, modifier = Modifier.weight(1f)) {
                    Icon(Icons.Filled.CreateNewFolder, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Compartir")
                }
                OutlinedButton(onClick = { showJoin = true }, modifier = Modifier.weight(1f)) {
                    Icon(Icons.Filled.QrCodeScanner, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Unirme")
                }
            }
        }
    }

    if (pickForShare) {
        DirectoryPicker(
            title = "¿Qué carpeta compartes?",
            onDismiss = { pickForShare = false },
            onPicked = { dir ->
                pickForShare = false
                scope.launch {
                    val outcome = withContext(Dispatchers.IO) {
                        runCatching { Repo.shareFolder(dir.absolutePath, dir.name) }
                    }
                    outcome.fold(
                        onSuccess = { codeOnScreen = dir.name to it },
                        onFailure = { error = it.message },
                    )
                }
            },
        )
    }

    codeOnScreen?.let { (label, code) ->
        CodeDialog(label = label, code = code, onDismiss = { codeOnScreen = null })
    }

    if (showJoin) {
        JoinDialog(
            onDismiss = { showJoin = false },
            onJoined = { showJoin = false },
            onError = { error = it },
        )
    }

    if (showSettings) {
        SettingsDialog(onDismiss = { showSettings = false }, onError = { error = it })
    }

    openFolder?.let { folder ->
        FolderDialog(
            folder = folder,
            onDismiss = { openFolder = null },
            onError = { error = it },
        )
    }
}

/**
 * Runs one engine call off the main thread and reports failure as a sentence.
 *
 * Every call crosses into Rust and blocks until the engine answers over a
 * socket, so doing this on the main thread would freeze the interface for as
 * long as the engine takes.
 */
private fun CoroutineScope.engineCall(
    onError: (String) -> Unit,
    onDone: () -> Unit = {},
    block: suspend () -> Unit,
) {
    launch(Dispatchers.IO) {
        val outcome = runCatching { block() }
        withContext(Dispatchers.Main) {
            outcome.fold(
                onSuccess = { onDone() },
                onFailure = { onError(it.message ?: "Algo no funcionó") },
            )
        }
    }
}

@Composable
private fun Banner(tone: androidx.compose.ui.graphics.Color, content: @Composable () -> Unit) {
    Box(
        Modifier
            .fillMaxWidth()
            .border(1.dp, tone, RoundedCornerShape(11.dp))
            .padding(12.dp),
    ) { content() }
}

@Composable
private fun InvitationBanner(invitation: Invitation, onDone: () -> Unit, onError: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var pickPath by remember { mutableStateOf(false) }

    Banner(tone = MaterialTheme.colorScheme.primary) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    buildString {
                        append(invitation.fromDeviceName)
                        append(
                            if (invitation.folder != null) " quiere compartir «${invitation.folder.label}»"
                            else " quiere conectarse con este dispositivo"
                        )
                    },
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    shortId(invitation.fromDeviceId),
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    scope.engineCall(onError, onDone = { busy = false; onDone() }) {
                        Repo.decline(invitation)
                    }
                },
            ) { Text("Rechazar") }
            Button(
                enabled = !busy,
                onClick = {
                    // A folder has to land somewhere; a bare device does not.
                    if (invitation.folder != null) {
                        pickPath = true
                    } else {
                        busy = true
                        scope.engineCall(onError, onDone = { busy = false; onDone() }) {
                            Repo.accept(invitation, null)
                        }
                    }
                },
            ) { Text("Aceptar") }
        }
    }

    if (pickPath && invitation.folder != null) {
        DirectoryPicker(
            title = "¿Dónde guardo «${invitation.folder.label}»?",
            onDismiss = { pickPath = false },
            onPicked = { dir ->
                pickPath = false
                scope.engineCall(onError, onDone) {
                    // Same rule as joining by code: picking the folder itself
                    // must not create a copy of it inside itself.
                    val target = Repo.resolveDestination(dir.absolutePath, invitation.folder.label)
                    File(target.path).mkdirs()
                    Repo.accept(invitation, target.path)
                }
            },
        )
    }
}

@Composable
private fun FolderRow(folder: SharedFolder, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(11.dp))
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(11.dp))
            .clickable(onClick = onClick)
            .padding(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StatusDot(folder.state)
        Spacer(Modifier.width(11.dp))
        Column(Modifier.weight(1f)) {
            Text(folder.label, fontWeight = FontWeight.Medium)
            Text(
                "${formatBytes(folder.bytes)} · ${peerSummary(folder.peers)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Column(horizontalAlignment = Alignment.End) {
            Text(
                stateLabel(folder.state),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // "Sincronizando 9%" says nothing about whether to wait for it.
            // How long is left, and how fast, is what does.
            remaining(folder)?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun StatusDot(state: FolderState) {
    val color = when (state) {
        FolderState.UpToDate -> MaterialTheme.colorScheme.primary
        is FolderState.Syncing -> MaterialTheme.colorScheme.primary
        is FolderState.Problem -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.outline
    }
    Box(Modifier.size(9.dp).clip(CircleShape).background(color))
}

@Composable
private fun CodeDialog(label: String, code: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val qr = rememberQr(code)
    var copied by remember { mutableStateOf(false) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Compartir «$label»") },
        text = {
            // While this is up, the other device is pointing a camera at it.
            KeepScreenReadable(bright = true)
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    "Escanea esto desde el otro dispositivo, o pásale el código.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                if (qr != null) {
                    Image(
                        bitmap = qr,
                        contentDescription = "Código QR para compartir $label",
                        modifier = Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(10.dp)),
                    )
                } else {
                    Text("No se pudo dibujar el QR", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }

                Spacer(Modifier.height(14.dp))
                // The same code as the QR, in a field rather than as text: it
                // can be selected and dragged out by hand when neither copying
                // nor sharing is what the moment calls for.
                OutlinedTextField(
                    value = code,
                    onValueChange = {},
                    readOnly = true,
                    label = { Text("Código") },
                    textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                    maxLines = 2,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = { copyToClipboard(context, code); copied = true }) {
                        Icon(
                            if (copied) Icons.Filled.Check else Icons.Filled.ContentCopy,
                            null,
                            Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        Text(if (copied) "Copiado" else "Copiar")
                    }
                    OutlinedButton(onClick = { shareCode(context, label, code) }) {
                        Icon(Icons.Filled.Share, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Compartir")
                    }
                }

                Spacer(Modifier.height(12.dp))
                Text(
                    "Quien lo use entrará solo, sin que aceptes nada más.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(10.dp))
                Text(
                    "Cualquiera con este código puede entrar en «$label». No lo publiques.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Cerrar") } },
    )
}

/**
 * Hands the code to whatever the user already uses to talk to themselves.
 *
 * Pairing usually happens between two devices in the same pair of hands, and
 * the code has to cross that gap somehow. Copying only helps if the other
 * device shares a clipboard; sending it through a chat is what most people
 * actually do.
 */
private fun shareCode(context: Context, label: String, code: String) {
    val intent = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_SUBJECT, "Código de HomeCloud para «$label»")
        putExtra(Intent.EXTRA_TEXT, code)
    }
    runCatching {
        context.startActivity(Intent.createChooser(intent, "Compartir el código"))
    }
}

@Composable
private fun JoinDialog(onDismiss: () -> Unit, onJoined: () -> Unit, onError: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf("") }
    var preview by remember { mutableStateOf<CodePreview?>(null) }
    var destination by remember { mutableStateOf<Destination?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    var pickPath by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var needsPassword by remember { mutableStateOf(false) }

    var scanning by remember { mutableStateOf(false) }

    // Reading the code as it is typed means a wrong one is caught before the
    // user commits to a destination.
    LaunchedEffect(code, password) {
        if (code.trim().length < 8) {
            preview = null
            destination = null
            problem = null
            return@LaunchedEffect
        }
        val outcome = withContext(Dispatchers.IO) {
            runCatching {
                val read = Repo.previewCode(code, password)
                // Somewhere sensible to start: the phone's own folder of that
                // name if it has one, and never a directory nested in itself.
                val guess = File(Environment.getExternalStorageDirectory(), read.folderLabel)
                val chosen = if (guess.isDirectory) guess else Environment.getExternalStorageDirectory()
                read to Repo.resolveDestination(chosen.absolutePath, read.folderLabel)
            }
        }
        outcome.fold(
            onSuccess = { (read, target) ->
                preview = read
                destination = target
                problem = null
                needsPassword = false
            },
            onFailure = {
                preview = null
                destination = null
                // A locked code is not a broken one: it needs one more thing typed.
                val locked = it.message?.contains("contraseña") == true
                needsPassword = locked
                problem = if (locked && password.isEmpty()) null else it.message
            },
        )
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Unirme a una carpeta") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                OutlinedButton(onClick = { scanning = true }, modifier = Modifier.fillMaxWidth()) {
                    Icon(Icons.Filled.QrCodeScanner, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("Escanear el QR del otro dispositivo")
                }
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it },
                    label = { Text("…o pega el código") },
                    placeholder = { Text("HC1…") },
                    minLines = 2,
                )
                if (needsPassword) {
                    Spacer(Modifier.height(10.dp))
                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text("Esta carpeta tiene contraseña") },
                        singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                problem?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
                val read = preview
                val target = destination
                if (read != null && target != null) {
                    Spacer(Modifier.height(12.dp))
                    Text("${read.deviceName} comparte «${read.folderLabel}»")
                    Spacer(Modifier.height(10.dp))
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(10.dp))
                            .padding(12.dp),
                    ) {
                        Text(
                            target.path,
                            fontFamily = FontFamily.Monospace,
                            style = MaterialTheme.typography.bodySmall,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            target.explanation,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (read.bytes != null && target.freeBytes != null) {
                            val missing = shortfall(read.bytes, target.freeBytes)
                            Spacer(Modifier.height(4.dp))
                            Text(
                                buildString {
                                    append("Ocupa ${formatBytes(read.bytes)}")
                                    append(" · quedan ${formatBytes(target.freeBytes)} libres")
                                    if (missing > 0) append(" · faltan ${formatBytes(missing)}")
                                },
                                style = MaterialTheme.typography.bodySmall,
                                color = if (missing > 0) MaterialTheme.colorScheme.error
                                else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Spacer(Modifier.height(8.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick = { pickPath = true }) {
                                Icon(Icons.Filled.Edit, null, Modifier.size(17.dp))
                                Spacer(Modifier.width(6.dp))
                                Text("Cambiar")
                            }
                            TextButton(onClick = {
                                // Flipping needs the directory the user meant,
                                // which for "inside" is the one shown and for
                                // "itself" is its parent.
                                val chosen =
                                    if (target.putsItInside) File(target.path).parentFile
                                    else File(target.path)
                                val wanted = if (target.putsItInside) "itself" else "inside"
                                scope.launch {
                                    val flipped = withContext(Dispatchers.IO) {
                                        runCatching {
                                            Repo.resolveDestination(
                                                (chosen ?: File(target.path)).absolutePath,
                                                read.folderLabel,
                                                wanted,
                                            )
                                        }
                                    }
                                    flipped.onSuccess { destination = it }
                                }
                            }) {
                                Text(
                                    if (target.putsItInside) "Usar esa carpeta tal cual"
                                    else "Crear una subcarpeta dentro"
                                )
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            val target = destination
            TextButton(
                enabled = target != null && !busy,
                onClick = {
                    busy = true
                    scope.engineCall(onError, onDone = { busy = false; onJoined() }) {
                        File(target!!.path).mkdirs()
                        Repo.redeemCode(code, target.path, password)
                    }
                },
            ) {
                val wontFit = doesNotFit(preview?.bytes, target?.freeBytes)
                Text(if (busy) "Conectando…" else if (wontFit) "Unirme igual" else "Unirme")
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )

    if (scanning) {
        QrScannerSheet(
            onScanned = { scanned ->
                code = scanned
                scanning = false
            },
            onClose = { scanning = false },
        )
    }

    if (pickPath) {
        val read = preview
        DirectoryPicker(
            title = "¿Dónde guardo «${read?.folderLabel ?: "la carpeta"}»?",
            onDismiss = { pickPath = false },
            onPicked = { dir ->
                pickPath = false
                if (read != null) {
                    scope.launch {
                        val resolved = withContext(Dispatchers.IO) {
                            runCatching { Repo.resolveDestination(dir.absolutePath, read.folderLabel) }
                        }
                        resolved.fold(
                            onSuccess = { destination = it },
                            onFailure = { onError(it.message ?: "") },
                        )
                    }
                }
            },
        )
    }
}

@Composable
private fun FolderDialog(folder: SharedFolder, onDismiss: () -> Unit, onError: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf<String?>(null) }
    // Read-only, the device list and the wifi rule are settings: looked at once
    // when a folder is set up and never again. The actions come first.
    var advancedOpen by remember { mutableStateOf(false) }
    val paused = folder.state == FolderState.Paused
    // What the user just asked for, until the next poll confirms it. These
    // used to close the whole sheet and then take a second and a half to show
    // the new value, so the box looked untouched and got ticked twice.
    var wantedMode by remember(folder.id) { mutableStateOf<String?>(null) }
    var wantedWifiOnly by remember(folder.id) { mutableStateOf<Boolean?>(null) }
    // Once the engine agrees, the guess has nothing left to say.
    LaunchedEffect(folder.mode, folder.wifiOnly) {
        if (wantedMode == folder.mode) wantedMode = null
        if (wantedWifiOnly == folder.wifiOnly) wantedWifiOnly = null
    }

    code?.let {
        CodeDialog(label = folder.label, code = it, onDismiss = onDismiss)
        return
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(folder.label) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(stateLabel(folder.state))
                remaining(folder)?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    buildString {
                        append("${folder.files} ficheros · ${formatBytes(folder.bytes)}")
                        folder.freeBytes?.let { append(" · ${formatBytes(it)} libres") }
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                // A folder stopped halfway still has a real size and a real
                // amount left; the engine simply refuses to say so while it
                // is paused, which used to leave "0 B" on screen.
                if (paused && folder.pendingBytes > 0) {
                    Text(
                        "Le faltan ${formatBytes(folder.pendingBytes)} por bajar.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                Spacer(Modifier.height(12.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(
                        onClick = { openFolder(context, folder.path, onError) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(Icons.Filled.FolderOpen, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Abrir")
                    }
                    TextButton(
                        onClick = {
                            scope.launch {
                                val outcome = withContext(Dispatchers.IO) {
                                    runCatching { Repo.codeFor(folder.id) }
                                }
                                outcome.fold(
                                    onSuccess = { code = it },
                                    onFailure = { onError(it.message ?: "") },
                                )
                            }
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(Icons.Filled.PersonAdd, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Añadir")
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(
                        onClick = {
                            scope.engineCall(onError, onDismiss) { Repo.rescan(folder.id) }
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(Icons.Filled.Refresh, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Revisar")
                    }
                    TextButton(
                        onClick = {
                            scope.engineCall(onError, onDismiss) {
                                Repo.setFolderPaused(folder.id, !paused)
                            }
                        },
                        modifier = Modifier.weight(1f),
                    ) {
                        Icon(
                            if (paused) Icons.Filled.PlayArrow else Icons.Filled.Pause,
                            null,
                            Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        Text(if (paused) "Reanudar" else "Pausar")
                    }
                }

                Spacer(Modifier.height(6.dp))
                TextButton(onClick = { advancedOpen = !advancedOpen }) {
                    Icon(
                        if (advancedOpen) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                        null,
                        Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    Text("Avanzado")
                }

                if (advancedOpen) {
                    Text("Qué hace esta copia", style = MaterialTheme.typography.labelMedium)
                    val mode = wantedMode ?: folder.mode
                    FOLDER_MODES.forEach { (value, words) ->
                        val (title, explanation) = words
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            RadioButton(
                                selected = mode == value,
                                onClick = {
                                    wantedMode = value
                                    // A guess that turned out wrong must not
                                    // outlive the attempt.
                                    scope.engineCall({ wantedMode = null; onError(it) }) {
                                        Repo.setFolderMode(folder.id, value)
                                    }
                                },
                            )
                            Column {
                                Text(title, style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    explanation,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    if (mode == "archive" && folder.extraBytes > 0) {
                        Text(
                            "Ahora mismo guarda ${formatBytes(folder.extraBytes)} que ya no están " +
                                "en los otros dispositivos.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Spacer(Modifier.height(6.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(
                            checked = wantedWifiOnly ?: folder.wifiOnly,
                            onCheckedChange = { wanted ->
                                wantedWifiOnly = wanted
                                scope.engineCall({ wantedWifiOnly = null; onError(it) }) {
                                    Repo.setFolderWifiOnly(folder.id, wanted)
                                }
                            },
                        )
                        Column {
                            Text("Solo con wifi", style = MaterialTheme.typography.bodyMedium)
                            Text(
                                buildString {
                                    append("Se detiene cuando la conexión se paga por datos.")
                                    if (folder.pausedByNetwork) {
                                        append(" Ahora mismo está detenida por eso.")
                                    }
                                },
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }

                    Spacer(Modifier.height(10.dp))
                    FolderPassword(folder = folder, onError = onError, onDone = onDismiss)

                    Spacer(Modifier.height(10.dp))
                    ShareLink(folder = folder, onError = onError)

                    Spacer(Modifier.height(8.dp))
                    Text("Dispositivos", style = MaterialTheme.typography.labelMedium)
                    if (folder.peers.isEmpty()) {
                        Text(
                            "Todavía no comparte con ningún dispositivo.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    folder.peers.forEach { peer ->
                        Spacer(Modifier.height(6.dp))
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Box(
                                Modifier.size(8.dp).clip(CircleShape).background(
                                    if (peer.connected) MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.outline
                                )
                            )
                            Spacer(Modifier.width(8.dp))
                            Text(peer.name, style = MaterialTheme.typography.bodyMedium)
                            Spacer(Modifier.width(6.dp))
                            Text(
                                shortId(peer.id),
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            // Whether the other end has finished. Without it
                            // this phone reads "Al día" while the laptop it
                            // shares with is still at four per cent.
                            peer.completion?.let { done ->
                                Spacer(Modifier.width(6.dp))
                                Text(
                                    if (done >= 100) "al día" else "$done%",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(10.dp))
                    DeletedFiles(folder = folder, onError = onError)

                    Spacer(Modifier.height(8.dp))
                    Text(
                        folder.path,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )

                    Spacer(Modifier.height(4.dp))
                    TextButton(onClick = {
                        // The files stay on the phone. Only the syncing stops.
                        scope.engineCall(onError, onDismiss) { Repo.stopSharing(folder.id) }
                    }) {
                        Icon(Icons.Filled.LinkOff, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Dejar de sincronizar")
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Cerrar") } },
    )
}

/**
 * The three things a copy of a folder can be.
 *
 * The last one is what turns another device into somewhere a full phone can
 * delete against: it takes everything, sends nothing back, and never carries
 * out a deletion, so the videos removed here stay there.
 */
private val FOLDER_MODES = listOf(
    "twoWay" to ("La misma carpeta en los dos sitios" to
        "Lo que cambies o borres aquí pasa a los demás, y al revés."),
    "receiveOnly" to ("Solo recibe" to
        "Recibe los cambios de los demás, pero nunca envía los suyos."),
    "archive" to ("Copia de seguridad: lo guarda todo" to
        "Recibe todo y no borra nunca. Si en el otro dispositivo se borran vídeos para hacer sitio, aquí siguen."),
)

/**
 * What was deleted, and how to get it back.
 *
 * Syncing a deletion is the one change that syncing again cannot undo, so
 * nothing is destroyed: on a computer the file lands in the system's recycle
 * bin, and on a phone — which has no bin an app may write to on its own — the
 * engine keeps a copy beside the files. This list reads whichever it is, and
 * recovering one puts it back in the folder, which sends it to the other
 * devices again.
 */
@Composable
private fun DeletedFiles(folder: SharedFolder, onError: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var open by remember(folder.id) { mutableStateOf(false) }
    var files by remember(folder.id) { mutableStateOf<List<DeletedFile>?>(null) }
    var busy by remember { mutableStateOf<String?>(null) }

    suspend fun look() {
        val found = withContext(Dispatchers.IO) { runCatching { Repo.deletedFiles(folder.id) } }
        found.fold(
            onSuccess = { files = it },
            onFailure = { onError(it.message ?: "No se pudo mirar en la papelera") },
        )
    }

    LaunchedEffect(open) { if (open) look() }

    if (!open) {
        TextButton(onClick = { open = true }) {
            Icon(Icons.Filled.Restore, null, Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text("Buscar ficheros borrados")
        }
        return
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        Text("Ficheros borrados", style = MaterialTheme.typography.labelMedium)
        Help(
            "Lo que otro dispositivo borre no se destruye aquí: se guarda una copia y aparece en " +
                "esta lista. Recuperar uno lo devuelve a la carpeta, y desde ahí vuelve solo al " +
                "resto de dispositivos.",
        )
    }

    val found = files
    when {
        found == null -> Text(
            "Mirando…",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        found.isEmpty() -> Text(
            "No hay nada borrado de esta carpeta.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        else -> found.forEach { file ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(file.name, style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "${formatBytes(file.bytes)} · ${timeAgo(file.deletedAt)}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                TextButton(
                    enabled = busy != file.id,
                    onClick = {
                        busy = file.id
                        scope.launch {
                            val outcome = withContext(Dispatchers.IO) {
                                runCatching { Repo.restoreDeleted(folder.id, file.id) }
                            }
                            busy = null
                            outcome.fold(
                                onSuccess = {
                                    android.widget.Toast
                                        .makeText(
                                            context,
                                            "«${file.name}» vuelve a estar en la carpeta",
                                            android.widget.Toast.LENGTH_SHORT,
                                        )
                                        .show()
                                    look()
                                },
                                onFailure = { onError(it.message ?: "No se pudo recuperar") },
                            )
                        }
                    },
                ) { Text(if (busy == file.id) "Recuperando…" else "Recuperar") }
            }
        }
    }
    TextButton(onClick = { open = false }) { Text("Ocultar") }
}

/**
 * Handing a folder out as a link, for people who will not install anything.
 *
 * There is no account behind it — a Cloudflare Quick Tunnel needs none —
 * which is the same reason it is temporary: a quick tunnel gets a new address
 * every time and Cloudflare promises no uptime, so pretending otherwise would
 * be a lie the app tells on the user's behalf. What is on screen instead is
 * the truth: a countdown to when it stops, and that closing HomeCloud stops
 * it early.
 */
@Composable
private fun ShareLink(folder: SharedFolder, onError: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var status by remember { mutableStateOf<LinkStatus?>(null) }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var secondsLeft by remember { mutableStateOf(0L) }

    LaunchedEffect(folder.id) {
        val existing = withContext(Dispatchers.IO) { runCatching { Repo.linkFor(folder.id) }.getOrNull() }
        status = existing
    }

    // Ticks only while a link is actually up: nothing to count down otherwise.
    LaunchedEffect(status) {
        val current = status ?: return@LaunchedEffect
        while (true) {
            val left = current.expiresAt - System.currentTimeMillis() / 1000
            secondsLeft = left
            // hcshare stops itself once its time is up; reporting a link past
            // that point would be this screen lying about something it can
            // check.
            if (left <= 0) {
                status = null
                return@LaunchedEffect
            }
            delay(1000)
        }
    }

    Row(verticalAlignment = Alignment.CenterVertically) {
        Text("Compartir por enlace", style = MaterialTheme.typography.labelMedium)
        Help(
            "Cualquiera con el enlace podrá ver y descargar lo que hay en esta carpeta, sin instalar nada. " +
                "Solo lectura: no podrá cambiar ni borrar nada. Caduca solo a las pocas horas, sin cuenta en " +
                "ningún sitio. Sin contraseña, lo abre quien tenga el enlace; con ella, la comprueba este " +
                "teléfono, no un tercero.",
        )
    }

    val current = status
    if (current != null && secondsLeft > 0) {
        Text(current.url, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
        Text(
            "Caduca en ${countdown(secondsLeft)}, o antes si cierras HomeCloud en este teléfono.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = {
                copyToClipboard(context, current.url)
                android.widget.Toast
                    .makeText(context, "Enlace copiado", android.widget.Toast.LENGTH_SHORT)
                    .show()
            }) {
                Icon(Icons.Filled.ContentCopy, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Copiar")
            }
            TextButton(onClick = { shareCode(context, folder.label, current.url) }) {
                Icon(Icons.Filled.Share, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Compartir")
            }
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    scope.launch {
                        val outcome = withContext(Dispatchers.IO) { runCatching { Repo.linkStop(folder.id) } }
                        busy = false
                        outcome.fold(onSuccess = { status = null }, onFailure = { onError(it.message ?: "") })
                    }
                },
            ) { Text("Dejar de compartir") }
        }
        return
    }

    OutlinedTextField(
        value = password,
        onValueChange = { password = it },
        label = { Text("Contraseña del enlace (opcional)") },
        singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        modifier = Modifier.fillMaxWidth(),
    )
    TextButton(
        enabled = !busy,
        onClick = {
            busy = true
            val auth = if (password.isBlank()) "" else "familia:${password.trim()}"
            scope.launch {
                val outcome = withContext(Dispatchers.IO) {
                    runCatching { Repo.linkStart(folder.id, folder.path, auth) }
                }
                busy = false
                outcome.fold(
                    onSuccess = {
                        status = it
                        // Creating the link and then making the user tap a
                        // second button to get it onto the clipboard is one
                        // step more than the moment calls for.
                        copyToClipboard(context, it.url)
                        android.widget.Toast
                            .makeText(context, "Enlace copiado", android.widget.Toast.LENGTH_SHORT)
                            .show()
                    },
                    onFailure = { onError(it.message ?: "No se pudo crear el enlace") },
                )
            }
        },
    ) {
        Icon(Icons.Filled.Link, null, Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Text(if (busy) "Creando el enlace…" else "Crear el enlace")
    }
}

/**
 * A small "?" next to a label, popping its explanation only on demand.
 *
 * The alternative — a paragraph sitting under every control — is how a
 * folder's sharing screen ends up reading like terms and conditions. Most
 * people never need the explanation; anyone who does gets the whole thing.
 */
@Composable
private fun Help(text: String) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }, modifier = Modifier.size(20.dp)) {
            Text(
                "?",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            Text(
                text,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .widthIn(max = 260.dp),
            )
        }
    }
}

private fun countdown(seconds: Long): String {
    val hours = seconds / 3600
    val minutes = (seconds % 3600) / 60
    return when {
        hours > 0 -> "$hours h $minutes min"
        minutes > 0 -> "$minutes min"
        else -> "$seconds s"
    }
}

/**
 * The password on a folder.
 *
 * Not a password the engine checks — there is nowhere in the protocol for one.
 * It is what this folder's pairing codes are encrypted with, so a code that
 * leaks is a code nobody can use. Hence the wording: it is about the code, not
 * about locking files away from a device that already syncs them.
 */
@Composable
private fun FolderPassword(folder: SharedFolder, onError: (String) -> Unit, onDone: () -> Unit) {
    val scope = rememberCoroutineScope()
    var editing by remember { mutableStateOf(false) }
    var value by remember { mutableStateOf("") }

    Text("Contraseña", style = MaterialTheme.typography.labelMedium)
    Text(
        if (folder.hasPassword)
            "Los códigos de esta carpeta van cifrados: sin la contraseña no sirven de nada."
        else
            "Sin contraseña. Cualquiera con un código de esta carpeta puede entrar.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )

    if (editing) {
        OutlinedTextField(
            value = value,
            onValueChange = { value = it },
            label = { Text("Una contraseña para esta carpeta") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(
                enabled = value.isNotBlank(),
                onClick = {
                    val chosen = value
                    scope.engineCall(onError, onDone) {
                        Repo.setFolderPassword(folder.id, chosen)
                    }
                },
            ) { Text("Guardar") }
            TextButton(onClick = { editing = false; value = "" }) { Text("Cancelar") }
        }
    } else {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = { editing = true }) {
                Text(if (folder.hasPassword) "Cambiar" else "Poner una contraseña")
            }
            if (folder.hasPassword) {
                TextButton(onClick = {
                    scope.engineCall(onError, onDone) { Repo.setFolderPassword(folder.id, "") }
                }) { Text("Quitarla") }
            }
        }
    }
}

/**
 * Opens the folder in whatever file manager the phone has.
 *
 * A `file://` URI to a directory is refused outright — no app declares a
 * handler for it, so the chooser came back empty and the button looked broken.
 * Android's only supported way in is a document URI from the storage provider,
 * and even then not every file manager accepts one, so this walks down a list
 * of decreasing ambition and only gives up once nothing is left to try.
 */
private fun openFolder(context: Context, path: String, onError: (String) -> Unit) {
    val documentId = documentIdFor(path)
    val treeUri = DocumentsContract.buildTreeDocumentUri(EXTERNAL_STORAGE_PROVIDER, documentId)

    val attempts = listOf(
        // What a file manager registers for: a directory, as a document.
        Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(
                DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId),
                DocumentsContract.Document.MIME_TYPE_DIR,
            )
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        },
        // Some managers only take the tree form.
        Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(treeUri, DocumentsContract.Document.MIME_TYPE_DIR)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        },
        // Not "open" so much as "browse from here", but it always exists: it is
        // the system's own picker, opened at this folder.
        Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            putExtra(DocumentsContract.EXTRA_INITIAL_URI, treeUri)
        },
    )

    for (intent in attempts) {
        val started = runCatching { context.startActivity(intent) }.isSuccess
        if (started) return
    }
    onError("Ningún gestor de archivos de este teléfono quiso abrir la carpeta. Está en $path")
}

private const val EXTERNAL_STORAGE_PROVIDER = "com.android.externalstorage.documents"

/**
 * `/storage/emulated/0/DCIM` becomes `primary:DCIM`, which is how the storage
 * provider names it. A path outside primary storage — an SD card — is left as
 * a bare document id, which the provider will simply not resolve; the fallback
 * chain then does its job.
 */
private fun documentIdFor(path: String): String {
    val roots = listOf("/storage/emulated/0/", "/sdcard/")
    val relative = roots.firstOrNull { path.startsWith(it) }?.let { path.removePrefix(it) }
    return if (relative != null) "primary:$relative" else "primary:"
}

@Composable
private fun SettingsDialog(onDismiss: () -> Unit, onError: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var settings by remember { mutableStateOf<Settings?>(null) }
    var tidy by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            runCatching { Repo.settings() }
                .onSuccess { loaded -> withContext(Dispatchers.Main) { settings = loaded } }
                .onFailure { e -> withContext(Dispatchers.Main) { onError(e.message ?: "") } }
        }
    }

    val current = settings
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Ajustes") },
        text = {
            if (current == null) {
                Text("Cargando…")
            } else {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    OutlinedTextField(
                        value = current.deviceName,
                        onValueChange = { settings = current.copy(deviceName = it) },
                        label = { Text("Nombre de este dispositivo") },
                        singleLine = true,
                    )
                    Text(
                        "Es el nombre que ven los demás dispositivos al conectarse.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(14.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(
                            checked = current.localNetworkOnly,
                            onCheckedChange = { settings = current.copy(localNetworkOnly = it) },
                        )
                        Column {
                            Text("Solo en mi red local")
                            Text(
                                "No se anuncia por internet ni usa repetidores.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                    Spacer(Modifier.height(14.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(
                            checked = current.deletionPolicy != "nothing",
                            onCheckedChange = { keep ->
                                settings = current.copy(
                                    // A phone has no system recycle bin to
                                    // point at, so keeping means the copies
                                    // the engine holds beside the files.
                                    deletionPolicy = if (keep) "copies" else "nothing",
                                )
                            },
                        )
                        Column {
                            Text("Guardar lo que se borre")
                            Text(
                                "Si otro dispositivo borra un fichero, aquí se guarda una copia y " +
                                    "puedes recuperarla desde la carpeta.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }

                    Spacer(Modifier.height(14.dp))
                    UpdateRow(onError = onError)

                    Spacer(Modifier.height(14.dp))
                    Text("Idioma", style = MaterialTheme.typography.labelMedium)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        listOf("es" to "Español", "en" to "Inglés").forEach { (code, name) ->
                            FilterChip(
                                selected = current.language == code,
                                onClick = { settings = current.copy(language = code) },
                                label = { Text(name) },
                            )
                        }
                    }

                    Spacer(Modifier.height(14.dp))
                    Text("Dispositivos olvidados", style = MaterialTheme.typography.labelMedium)
                    Text(
                        "Reinstalar la app le da al teléfono una identidad nueva, y la vieja se " +
                            "queda en la lista con el mismo nombre. Esto quita las que ya no " +
                            "comparten ninguna carpeta. No borra ficheros.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    TextButton(onClick = {
                        tidy = "Limpiando…"
                        scope.launch {
                            val outcome = withContext(Dispatchers.IO) {
                                runCatching { Repo.forgetUnusedDevices() }
                            }
                            tidy = outcome.fold(
                                onSuccess = {
                                    if (it.isEmpty()) "No había ninguno que sobrara."
                                    else "Se quitaron ${it.size}: ${it.joinToString(", ")}."
                                },
                                onFailure = { it.message ?: "No se pudo limpiar" },
                            )
                        }
                    }) {
                        Icon(Icons.Filled.CleaningServices, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Limpiar dispositivos que no comparten nada")
                    }
                    tidy?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    Spacer(Modifier.height(10.dp))
                    Text("Este dispositivo", style = MaterialTheme.typography.labelMedium)
                    Text(
                        current.deviceId,
                        Modifier.clickable { copyToClipboard(context, current.deviceId) },
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                    Text(
                        "Motor: Syncthing ${current.engineVersion}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = current != null,
                onClick = {
                    val toSave = current!!
                    scope.engineCall(onError, onDismiss) { Repo.saveSettings(toSave) }
                },
            ) { Text("Guardar") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}

/**
 * Looking for a newer HomeCloud, and installing it.
 *
 * The whole flow lives in one row because it is one thought: what version is
 * this, is there a newer one, install it. Android's three-step permission
 * dance around installing a package is handled by [Updater]; the only part
 * that surfaces here is being sent to the right settings screen when the
 * permission is missing, because a button that silently does nothing is worse
 * than one that explains itself.
 */
@Composable
private fun UpdateRow(onError: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<String?>(null) }
    var found by remember { mutableStateOf<Updater.Available?>(null) }
    var busy by remember { mutableStateOf(false) }

    val current = remember {
        runCatching {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
        }.getOrDefault("")
    }

    Text("Actualizaciones", style = MaterialTheme.typography.labelMedium)
    Text(
        "Tienes la versión $current. Se busca en las publicaciones de GitHub; " +
            "la instalación la confirmas tú.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    // Once an update is found, offering "search again" next to "install this
    // now" is two buttons fighting for a dialog's width for no reason —
    // that crowding is what was breaking the label into two lines.
    val available = found
    if (available == null) {
        TextButton(
            enabled = !busy,
            onClick = {
                busy = true
                state = "Comprobando…"
                scope.launch {
                    val outcome = withContext(Dispatchers.IO) { Updater.check(current) }
                    busy = false
                    outcome.fold(
                        onSuccess = { result ->
                            if (result.hasUpdate) {
                                found = result
                                state = null
                            } else {
                                found = null
                                state = "Ya tienes la última versión ($current)."
                            }
                        },
                        onFailure = { state = "No se pudo comprobar: ${it.message}" },
                    )
                }
            },
        ) { Text(if (busy) "Comprobando…" else "Buscar actualizaciones", maxLines = 1) }
    } else {
        Button(
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
            onClick = {
                // Checked before downloading tens of megabytes that could
                // not be installed at the end of it.
                if (!Updater.canInstall(context)) {
                    state = "Permite instalar apps de HomeCloud y vuelve a intentarlo."
                    runCatching { context.startActivity(Updater.installPermissionIntent(context)) }
                    return@Button
                }
                busy = true
                scope.launch {
                    val outcome = withContext(Dispatchers.IO) {
                        runCatching {
                            Updater.download(context, available.apkUrl) { fraction ->
                                state = "Descargando… ${(fraction * 100).toInt()}%"
                            }
                        }
                    }
                    busy = false
                    outcome.fold(
                        onSuccess = { apk ->
                            state = "Confirma la instalación."
                            runCatching { context.startActivity(Updater.installIntent(context, apk)) }
                                .onFailure { onError(it.message ?: "No se pudo abrir el instalador") }
                        },
                        onFailure = { state = "No se pudo descargar: ${it.message}" },
                    )
                }
            },
        ) { Text("Actualizar a ${available.latest}", maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
    state?.let {
        Text(
            it,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private fun copyToClipboard(context: Context, text: String) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("HomeCloud", text))
}
