package app.readme

import org.json.JSONArray
import org.json.JSONObject

data class ReaderDoc(
    val title: String?,
    val author: String?,
    val published: String?,
    val site: String?,
    val words: Int,
    val blocks: List<ReaderBlock>,
)

sealed interface ReaderBlock {
    data class Heading(val level: Int, val text: String) : ReaderBlock
    data class Paragraph(val text: String) : ReaderBlock
    data class ListBlock(val ordered: Boolean, val items: List<String>) : ReaderBlock
    data class Quote(val text: String) : ReaderBlock
    data class Code(val text: String) : ReaderBlock
    data class Image(val src: String, val alt: String?, val caption: String?) : ReaderBlock
    data class Table(val rows: List<List<String>>) : ReaderBlock
    data object Unknown : ReaderBlock
}

private fun JSONObject.str(key: String): String? =
    if (isNull(key)) null else optString(key).takeIf { it.isNotBlank() }

private fun JSONArray.strings(): List<String> = List(length()) { getString(it) }

/** Converte o JSON do núcleo Rust (serde) no modelo de tela. */
fun parseDocument(json: String): ReaderDoc {
    val root = JSONObject(json)
    val meta = root.getJSONObject("meta")
    val blocks = root.getJSONArray("blocks")
    val parsed = List(blocks.length()) { i ->
        val b = blocks.getJSONObject(i)
        when (b.getString("type")) {
            "heading" -> ReaderBlock.Heading(b.getInt("level"), b.getString("text"))
            "paragraph" -> ReaderBlock.Paragraph(b.getString("text"))
            "list" -> ReaderBlock.ListBlock(b.getBoolean("ordered"), b.getJSONArray("items").strings())
            "quote" -> ReaderBlock.Quote(b.getString("text"))
            "code" -> ReaderBlock.Code(b.getString("text"))
            "image" -> ReaderBlock.Image(b.getString("src"), b.str("alt"), b.str("caption"))
            "table" -> {
                val rows = b.getJSONArray("rows")
                ReaderBlock.Table(List(rows.length()) { r -> rows.getJSONArray(r).strings() })
            }
            else -> ReaderBlock.Unknown
        }
    }
    val words = parsed.sumOf { block ->
        when (block) {
            is ReaderBlock.Heading -> block.text.split(Regex("\\s+")).size
            is ReaderBlock.Paragraph -> block.text.split(Regex("\\s+")).size
            is ReaderBlock.Quote -> block.text.split(Regex("\\s+")).size
            is ReaderBlock.Code -> block.text.split(Regex("\\s+")).size
            is ReaderBlock.ListBlock -> block.items.sumOf { it.split(Regex("\\s+")).size }
            else -> 0
        }
    }
    return ReaderDoc(
        title = meta.str("title"),
        author = meta.str("author"),
        published = meta.str("published"),
        site = meta.str("site"),
        words = words,
        blocks = parsed,
    )
}
