package app.readme

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.readme_ffi.readUrlJson

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val initial = intent?.dataString ?: ""
        setContent {
            MaterialTheme {
                ReaderScreen(initialUrl = initial)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(initialUrl: String) {
    var url by remember { mutableStateOf(initialUrl) }
    var state by remember { mutableStateOf<ReaderState>(ReaderState.Idle) }
    val scope = rememberCoroutineScope()

    fun load(target: String) {
        val clean = target.trim().let { if (it.startsWith("http")) it else "https://$it" }
        state = ReaderState.Loading
        scope.launch {
            state = try {
                val json = withContext(Dispatchers.IO) { readUrlJson(clean) }
                ReaderState.Ready(parseDocument(json))
            } catch (e: Exception) {
                ReaderState.Error(e.message ?: "Erro desconhecido")
            }
        }
    }

    LaunchedEffect(initialUrl) { if (initialUrl.isNotBlank()) load(initialUrl) }

    Scaffold(
        topBar = {
            TopAppBar(title = { Text("read.me", fontWeight = FontWeight.SemiBold) })
        }
    ) { padding ->
        Column(Modifier.padding(padding).padding(horizontal = 16.dp).fillMaxSize()) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    singleLine = true,
                    placeholder = { Text("Digite uma URL") },
                    keyboardActions = KeyboardActions(onGo = { load(url) }),
                    modifier = Modifier.weight(1f)
                )
                Spacer(Modifier.width(8.dp))
                Button(onClick = { load(url) }) { Text("Ler") }
            }
            Spacer(Modifier.height(12.dp))

            when (val s = state) {
                ReaderState.Idle -> Text("Cole uma URL para abrir em modo leitura.", modifier = Modifier.padding(8.dp))
                ReaderState.Loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
                is ReaderState.Error -> Text("Não foi possível abrir: ${s.message}", color = MaterialTheme.colorScheme.error)
                is ReaderState.Ready -> ArticleView(s.doc)
            }
        }
    }
}

sealed interface ReaderState {
    data object Idle : ReaderState
    data object Loading : ReaderState
    data class Error(val message: String) : ReaderState
    data class Ready(val doc: ReaderDoc) : ReaderState
}

@Composable
fun ArticleView(doc: ReaderDoc) {
    LazyColumn(verticalArrangement = Arrangement.spacedBy(14.dp), contentPadding = PaddingValues(bottom = 32.dp)) {
        doc.title?.let { item { Text(it, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold) } }
        val meta = listOfNotNull(doc.site, doc.author, doc.published).joinToString(" · ")
        if (meta.isNotBlank()) item { Text(meta, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline) } 
        item { Text("≈ ${maxOf(1, doc.words / 220)} min de leitura", style = MaterialTheme.typography.labelSmall) }
        items(doc.blocks) { BlockView(it) }
    }
}

@Composable
fun BlockView(b: ReaderBlock) {
    SelectionContainer {
        when (b) {
            is ReaderBlock.Heading -> Text(b.text, style = if (b.level <= 2) MaterialTheme.typography.titleLarge else MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            is ReaderBlock.Paragraph -> Text(b.text, style = MaterialTheme.typography.bodyLarge.copy(fontSize = 17.sp, lineHeight = 27.sp))
            is ReaderBlock.Quote -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.fillMaxWidth()) {
                Text(b.text, modifier = Modifier.padding(12.dp), fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)
            }
            is ReaderBlock.ListBlock -> Column {
                b.items.forEachIndexed { i, item ->
                    Text((if (b.ordered) "${i + 1}. " else "• ") + item, style = MaterialTheme.typography.bodyLarge)
                }
            }
            is ReaderBlock.Code -> Surface(color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.fillMaxWidth()) {
                Text(b.text, modifier = Modifier.padding(10.dp), fontFamily = FontFamily.Monospace, fontSize = 13.sp)
            }
            is ReaderBlock.Image -> Column {
                Text("[imagem] ${b.alt ?: b.src}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
                b.caption?.let { Text(it, style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Start) }
            }
            is ReaderBlock.Table -> Column {
                b.rows.forEach { row -> Text(row.joinToString("  |  "), fontFamily = FontFamily.Monospace, fontSize = 13.sp) }
            }
            is ReaderBlock.Unknown -> Unit
        }
    }
}
