package dev.lucasmoy.homecloud

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import java.io.File

/**
 * A plain directory browser over shared storage.
 *
 * Android's own picker hands back a permission-scoped URI, and the sync engine
 * is a separate process that can only open real paths — so the app browses the
 * filesystem itself. That is what the all-files permission is for, and why
 * HomeCloud is sideloaded rather than listed on Play.
 */
@Composable
fun DirectoryPicker(
    title: String,
    startAt: File = File("/storage/emulated/0"),
    onDismiss: () -> Unit,
    onPicked: (File) -> Unit,
) {
    var current by remember { mutableStateOf(if (startAt.isDirectory) startAt else File("/storage/emulated/0")) }
    val children = remember(current) {
        current.listFiles()
            ?.filter { it.isDirectory && !it.name.startsWith(".") }
            ?.sortedBy { it.name.lowercase() }
            ?: emptyList()
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                Text(
                    current.absolutePath.removePrefix("/storage/emulated/0").ifEmpty { "Almacenamiento interno" },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                LazyColumn(Modifier.heightIn(max = 320.dp)) {
                    if (current.parentFile != null && current.absolutePath != "/storage/emulated/0") {
                        item {
                            Text(
                                "⬑  Subir",
                                Modifier
                                    .fillMaxWidth()
                                    .clickable { current = current.parentFile!! }
                                    .padding(vertical = 12.dp),
                            )
                        }
                    }
                    items(children) { dir ->
                        Text(
                            dir.name,
                            Modifier
                                .fillMaxWidth()
                                .clickable { current = dir }
                                .padding(vertical = 12.dp),
                        )
                    }
                    if (children.isEmpty()) {
                        item {
                            Text(
                                "No hay subcarpetas aquí.",
                                Modifier.padding(vertical = 12.dp),
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onPicked(current) }) { Text("Usar esta carpeta") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
