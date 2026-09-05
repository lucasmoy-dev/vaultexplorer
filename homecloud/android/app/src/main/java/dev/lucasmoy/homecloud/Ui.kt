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
import androidx.compose.material.icons.filled.CleaningServices
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.LinkOff
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
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
    var pairing by remember { mutableStateOf<PairingWindow?>(null) }
    var ready by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    var showSettings by remember { mutableStateOf(false) }
    var showJoin by remember { mutableStateOf(false) }
    var pickForShare by remember { mutableStateOf(false) }
    var shareCode by remember { mutableStateOf<Pair<String, String>?>(null) }
    var openFolder by remember { mutableStateOf<SharedFolder?>(null) }

    suspend fun refresh() = withContext(Dispatchers.IO) {
        runCatching {
            val f = Repo.folders()
            val i = Repo.invitations()
            val w = Repo.pairingWindow()
            withContext(Dispatchers.Main) {
                folders = f
                invitations = i
                pairing = w
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

            pairing?.let { window ->
                Banner(tone = MaterialTheme.colorScheme.primary) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Filled.Schedule, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text(
                            "Esperando a que otro dispositivo entre en «${window.folderLabel}». " +
                                "Quedan ${(window.secondsLeft + 59) / 60} min.",
                            Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall,
                        )
                        TextButton(onClick = {
                            scope.launch {
                                withContext(Dispatchers.IO) { runCatching { Repo.closePairingWindow() } }
                                refresh()
                            }
                        }) { Text("Ya está") }
                    }
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
                Box(Modifier.weight(1f), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(12.dp))
                        Text("Arrancando…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            } else if (folders.isEmpty()) {
                Box(Modifier.weight(1f), contentAlignment = Alignment.Center) {
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
                        onSuccess = { shareCode = dir.name to it },
                        onFailure = { error = it.message },
                    )
                }
            },
        )
    }

    shareCode?.let { (label, code) ->
        CodeDialog(label = label, code = code, onDismiss = { shareCode = null })
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
        Text(
            stateLabel(folder.state),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
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
    var secondsLeft by remember { mutableStateOf<Long?>(null) }

    // The window is open for as long as this is on screen, so it is counted
    // down here rather than left as something the user has to know about.
    LaunchedEffect(Unit) {
        while (true) {
            val window = withContext(Dispatchers.IO) { runCatching { Repo.pairingWindow() }.getOrNull() }
            secondsLeft = window?.secondsLeft
            delay(1000)
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Compartir «$label»") },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    "Escanea esto desde el otro dispositivo, o pega el código.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                if (qr != null) {
                    Image(
                        bitmap = qr,
                        contentDescription = "Código QR para compartir $label",
                        modifier = Modifier.size(220.dp).clip(RoundedCornerShape(10.dp)),
                    )
                } else {
                    Text("No se pudo dibujar el QR", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    code,
                    fontFamily = FontFamily.Monospace,
                    style = MaterialTheme.typography.bodySmall,
                )
                secondsLeft?.let {
                    Spacer(Modifier.height(10.dp))
                    Text(
                        "Entrará solo, sin que aceptes nada más, durante ${countdown(it)}.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Spacer(Modifier.height(10.dp))
                Text(
                    "Cualquiera con este código puede entrar en «$label». No lo publiques.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { copyToClipboard(context, code); onDismiss() }) {
                Icon(Icons.Filled.ContentCopy, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Copiar")
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cerrar") } },
    )
}

private fun countdown(seconds: Long): String {
    val minutes = seconds / 60
    val rest = seconds % 60
    return if (minutes == 0L) "$rest s" else "$minutes:${rest.toString().padStart(2, '0')} min"
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

    val scan = rememberQrScanner { scanned -> code = scanned }

    // Reading the code as it is typed means a wrong one is caught before the
    // user commits to a destination.
    LaunchedEffect(code) {
        if (code.trim().length < 8) {
            preview = null
            destination = null
            problem = null
            return@LaunchedEffect
        }
        val outcome = withContext(Dispatchers.IO) {
            runCatching {
                val read = Repo.previewCode(code)
                // Somewhere sensible to start: the phone's own folder of that
                // name if it has one, and never a directory nested in itself.
                val guess = File(Environment.getExternalStorageDirectory(), read.folderLabel)
                val chosen = if (guess.isDirectory) guess else Environment.getExternalStorageDirectory()
                read to Repo.resolveDestination(chosen.absolutePath, read.folderLabel)
            }
        }
        outcome.fold(
            onSuccess = { (read, target) -> preview = read; destination = target; problem = null },
            onFailure = { preview = null; destination = null; problem = it.message },
        )
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Unirme a una carpeta") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                OutlinedButton(onClick = scan, modifier = Modifier.fillMaxWidth()) {
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
                        Repo.redeemCode(code, target.path)
                    }
                },
            ) { Text(if (busy) "Conectando…" else "Unirme") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )

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
    val scope = rememberCoroutineScope()
    var code by remember { mutableStateOf<String?>(null) }
    val paused = folder.state == FolderState.Paused

    code?.let {
        CodeDialog(label = folder.label, code = it, onDismiss = onDismiss)
        return
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(folder.label) },
        text = {
            Column {
                Text(stateLabel(folder.state))
                Text(
                    "${folder.files} ficheros · ${formatBytes(folder.bytes)}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(6.dp))
                Text(
                    folder.path,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
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
                    }
                }
                Spacer(Modifier.height(12.dp))
                TextButton(onClick = {
                    scope.launch {
                        val outcome = withContext(Dispatchers.IO) {
                            runCatching { Repo.codeFor(folder.id) }
                        }
                        outcome.fold(
                            onSuccess = { code = it },
                            onFailure = { onError(it.message ?: "") },
                        )
                    }
                }) {
                    Icon(Icons.Filled.PersonAdd, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Añadir otro dispositivo")
                }
                TextButton(onClick = {
                    scope.engineCall(onError, onDismiss) { Repo.setFolderPaused(folder.id, !paused) }
                }) {
                    Icon(
                        if (paused) Icons.Filled.PlayArrow else Icons.Filled.Pause,
                        null,
                        Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(6.dp))
                    Text(if (paused) "Reanudar" else "Pausar")
                }
                TextButton(onClick = {
                    // The files stay on the phone. Only the syncing stops.
                    scope.engineCall(onError, onDismiss) { Repo.stopSharing(folder.id) }
                }) {
                    Icon(Icons.Filled.LinkOff, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Dejar de sincronizar")
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Cerrar") } },
    )
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

private fun copyToClipboard(context: Context, text: String) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText("HomeCloud", text))
}
