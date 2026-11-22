package cn.lemonit.galaxy.ui.ai

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.R
import cn.lemonit.galaxy.ui.theme.Galaxy
import kotlinx.coroutines.delay

/** 模型回复用到的 Markdown 子集：标题、段落、有序 / 无序列表、引用、分隔线、代码块（带语言名）、表格。 */
sealed class MdBlock {
    data class Heading(val level: Int, val text: String) : MdBlock()
    data class Paragraph(val text: String) : MdBlock()
    data class ListItem(val marker: String, val text: String, val depth: Int) : MdBlock()
    data class Quote(val text: String) : MdBlock()
    data class Code(val language: String, val text: String) : MdBlock()
    data class Table(val header: List<String>, val rows: List<List<String>>) : MdBlock()
    data object Rule : MdBlock()
}

/** 行内片段：粗体、斜体、行内代码、链接、普通文字。 */
sealed class MdSpan {
    data class Plain(val text: String) : MdSpan()
    data class Bold(val text: String) : MdSpan()
    data class Italic(val text: String) : MdSpan()
    data class Code(val text: String) : MdSpan()
    data class Link(val text: String, val url: String) : MdSpan()
}

object Markdown {
    private val ORDERED = Regex("^(\\d+)[.)]\\s+")
    private val TABLE_SEP = Regex("^\\|?\\s*:?-+:?\\s*(\\|\\s*:?-+:?\\s*)*\\|?\\s*$")
    private val BARE_URL = Regex("https?://[^\\s)<>\\]]+")

    fun parse(md: String): List<MdBlock> {
        val out = mutableListOf<MdBlock>()
        val lines = md.replace("\r\n", "\n").split('\n')
        val para = StringBuilder()
        fun flush() {
            if (para.isNotBlank()) out.add(MdBlock.Paragraph(para.toString().trim()))
            para.clear()
        }
        var i = 0
        while (i < lines.size) {
            val line = lines[i]
            val t = line.trimStart()
            val indent = line.length - t.length
            when {
                t.startsWith("```") -> {
                    flush()
                    val lang = t.removePrefix("```").trim()
                    val code = StringBuilder()
                    i++
                    // 流式输出时代码块可能还没闭合：一直取到结尾
                    while (i < lines.size && !lines[i].trimStart().startsWith("```")) {
                        if (code.isNotEmpty()) code.append('\n')
                        code.append(lines[i])
                        i++
                    }
                    out.add(MdBlock.Code(lang, code.toString().trimEnd()))
                }
                t.startsWith("|") && i + 1 < lines.size && TABLE_SEP.matches(lines[i + 1].trim()) -> {
                    flush()
                    val header = cells(t)
                    val rows = mutableListOf<List<String>>()
                    i += 2
                    while (i < lines.size && lines[i].trimStart().startsWith("|")) {
                        val c = cells(lines[i].trim())
                        rows.add(List(header.size) { k -> c.getOrElse(k) { "" } })
                        i++
                    }
                    i--
                    out.add(MdBlock.Table(header, rows))
                }
                t.startsWith("#") && t.trimStart('#').let { it.isEmpty() || it.startsWith(" ") } -> {
                    flush()
                    val level = t.takeWhile { it == '#' }.length.coerceAtMost(6)
                    out.add(MdBlock.Heading(level, t.drop(level).trim()))
                }
                t == "---" || t == "***" || t == "___" -> {
                    flush()
                    out.add(MdBlock.Rule)
                }
                t.startsWith(">") -> {
                    flush()
                    out.add(MdBlock.Quote(t.removePrefix(">").trim()))
                }
                t.startsWith("- ") || t.startsWith("* ") || t.startsWith("+ ") -> {
                    flush()
                    out.add(MdBlock.ListItem("•", t.drop(2).trim(), indent / 2))
                }
                ORDERED.containsMatchIn(t) -> {
                    flush()
                    val m = ORDERED.find(t)!!
                    out.add(MdBlock.ListItem(m.groupValues[1] + ".", t.drop(m.value.length).trim(), indent / 2))
                }
                t.isEmpty() -> flush()
                else -> {
                    if (para.isNotEmpty()) para.append('\n')
                    para.append(line.trim())
                }
            }
            i++
        }
        flush()
        return out
    }

    private fun cells(row: String): List<String> {
        var r = row.trim()
        if (r.startsWith("|")) r = r.drop(1)
        if (r.endsWith("|")) r = r.dropLast(1)
        return r.split('|').map { it.trim() }
    }

    fun spans(text: String): List<MdSpan> {
        val out = mutableListOf<MdSpan>()
        val plain = StringBuilder()
        fun flushPlain() {
            if (plain.isEmpty()) return
            // 普通文字里的裸网址也做成链接
            var last = 0
            val s = plain.toString()
            BARE_URL.findAll(s).forEach { m ->
                if (m.range.first > last) out.add(MdSpan.Plain(s.substring(last, m.range.first)))
                val url = m.value.trimEnd('.', ',', ';', ':', '。', '，')
                out.add(MdSpan.Link(url, url))
                last = m.range.first + url.length
            }
            if (last < s.length) out.add(MdSpan.Plain(s.substring(last)))
            plain.clear()
        }
        var i = 0
        while (i < text.length) {
            val c = text[i]
            when {
                text.startsWith("**", i) && text.indexOf("**", i + 2).let { it > i + 2 } -> {
                    val end = text.indexOf("**", i + 2)
                    flushPlain()
                    out.add(MdSpan.Bold(text.substring(i + 2, end)))
                    i = end + 2
                }
                c == '`' && text.indexOf('`', i + 1) > i -> {
                    val end = text.indexOf('`', i + 1)
                    flushPlain()
                    out.add(MdSpan.Code(text.substring(i + 1, end)))
                    i = end + 1
                }
                c == '[' -> {
                    val close = text.indexOf("](", i + 1)
                    val end = if (close > 0) text.indexOf(')', close + 2) else -1
                    if (close > 0 && end > 0 && !text.substring(i + 1, close).contains('\n')) {
                        flushPlain()
                        out.add(MdSpan.Link(text.substring(i + 1, close), text.substring(close + 2, end).trim()))
                        i = end + 1
                    } else {
                        plain.append(c)
                        i++
                    }
                }
                c == '*' && i + 1 < text.length && !text[i + 1].isWhitespace() && text.indexOf('*', i + 1).let { it > i + 1 && !text[it - 1].isWhitespace() } -> {
                    val end = text.indexOf('*', i + 1)
                    flushPlain()
                    out.add(MdSpan.Italic(text.substring(i + 1, end)))
                    i = end + 1
                }
                else -> {
                    plain.append(c)
                    i++
                }
            }
        }
        flushPlain()
        return out
    }
}

@Composable
private fun inline(text: String, cursor: Color? = null): AnnotatedString {
    val codeBg = Galaxy.colors.codeBackground
    val codeFg = MaterialTheme.colorScheme.primary
    val linkStyle = TextLinkStyles(SpanStyle(color = MaterialTheme.colorScheme.primary, textDecoration = TextDecoration.Underline))
    return buildAnnotatedString {
        Markdown.spans(text).forEach { s ->
            when (s) {
                is MdSpan.Plain -> append(s.text)
                is MdSpan.Bold -> withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(s.text) }
                is MdSpan.Italic -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(s.text) }
                is MdSpan.Code -> withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeBg, color = codeFg)) { append(s.text) }
                is MdSpan.Link -> withLink(LinkAnnotation.Url(s.url, linkStyle)) { append(s.text) }
            }
        }
        if (cursor != null) withStyle(SpanStyle(color = cursor)) { append(" ▍") }
    }
}

/**
 * 按 Markdown 显示模型回复。[streaming] 为真时末尾一个闪烁的光标。
 */
@Composable
fun MarkdownText(md: String, modifier: Modifier = Modifier, color: Color = MaterialTheme.colorScheme.onSurface, streaming: Boolean = false) {
    val blocks = remember(md) { Markdown.parse(md) }
    var blink by remember { mutableStateOf(true) }
    if (streaming) {
        LaunchedEffect(Unit) {
            while (true) {
                delay(530)
                blink = !blink
            }
        }
    }
    val cursorColor = if (!streaming) null else if (blink) MaterialTheme.colorScheme.primary else Color.Transparent
    val lastText = blocks.indexOfLast { it is MdBlock.Paragraph || it is MdBlock.ListItem || it is MdBlock.Heading || it is MdBlock.Quote }
    val cursorOnLast = streaming && lastText == blocks.lastIndex
    val body = MaterialTheme.typography.bodyLarge.copy(lineHeight = MaterialTheme.typography.bodyLarge.lineHeight)
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        blocks.forEachIndexed { idx, b ->
            val cur = if (cursorOnLast && idx == blocks.lastIndex) cursorColor else null
            when (b) {
                is MdBlock.Heading -> Text(
                    inline(b.text, cur),
                    style = when (b.level) {
                        1 -> MaterialTheme.typography.titleLarge
                        2 -> MaterialTheme.typography.titleMedium
                        else -> MaterialTheme.typography.titleSmall
                    },
                    fontWeight = FontWeight.SemiBold,
                    color = color,
                    modifier = Modifier.padding(top = 4.dp),
                )
                is MdBlock.Paragraph -> Text(inline(b.text, cur), style = body, color = color)
                is MdBlock.ListItem -> Row(Modifier.padding(start = (b.depth * 16).dp)) {
                    Text(
                        b.marker, style = body, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.width(if (b.marker == "•") 18.dp else 24.dp),
                    )
                    Text(inline(b.text, cur), style = body, color = color)
                }
                is MdBlock.Quote -> Row(Modifier.height(IntrinsicSize.Min)) {
                    Box(Modifier.width(3.dp).fillMaxHeight().clip(RoundedCornerShape(2.dp)).background(MaterialTheme.colorScheme.outlineVariant))
                    Spacer(Modifier.width(10.dp))
                    Text(inline(b.text, cur), style = body, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                MdBlock.Rule -> HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                is MdBlock.Code -> CodeBlock(b)
                is MdBlock.Table -> TableBlock(b, color)
            }
        }
        if (streaming && !cursorOnLast) Text(inline("", cursorColor), style = body)
    }
}

@Composable
private fun CodeBlock(b: MdBlock.Code) {
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(1500)
            copied = false
        }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(10.dp))
            .background(Galaxy.colors.codeBackground),
    ) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                b.language.ifEmpty { "text" },
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            IconButton(
                onClick = {
                    (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("code", b.text))
                    copied = true
                },
                modifier = Modifier.size(36.dp),
            ) {
                Icon(
                    if (copied) Icons.Rounded.Check else Icons.Rounded.ContentCopy,
                    stringResource(if (copied) R.string.copied else R.string.copy),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Text(
            b.text,
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = MaterialTheme.colorScheme.onSurface,
            softWrap = false,
            modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(12.dp),
        )
    }
}

@Composable
private fun TableBlock(t: MdBlock.Table, color: Color) {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val headStyle: TextStyle = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold)
    val cellStyle: TextStyle = MaterialTheme.typography.bodyMedium
    // 每列宽度取该列最长内容，限制在 56–240dp，内容更长时换行
    val widths = remember(t) {
        t.header.indices.map { c ->
            val texts = listOf(t.header[c] to headStyle) + t.rows.map { it[c] to cellStyle }
            val px = texts.maxOf { (s, st) -> measurer.measure(s, st, maxLines = 1).size.width }
            with(density) { px.toDp() + 24.dp }.coerceIn(56.dp, 240.dp)
        }
    }
    Box(
        Modifier
            .clip(RoundedCornerShape(10.dp))
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(10.dp))
            .horizontalScroll(rememberScrollState()),
    ) {
        Column {
            Row(Modifier.height(IntrinsicSize.Min).background(MaterialTheme.colorScheme.surfaceContainerHigh)) {
                t.header.forEachIndexed { c, h ->
                    Text(inline(h), style = headStyle, color = color, modifier = Modifier.width(widths[c]).padding(horizontal = 12.dp, vertical = 8.dp))
                }
            }
            val total = widths.fold(0.dp) { a, w -> a + w }
            t.rows.forEach { r ->
                HorizontalDivider(Modifier.width(total), color = MaterialTheme.colorScheme.outlineVariant)
                Row(Modifier.height(IntrinsicSize.Min)) {
                    r.forEachIndexed { c, v ->
                        Text(inline(v), style = cellStyle, color = color, modifier = Modifier.width(widths[c]).padding(horizontal = 12.dp, vertical = 8.dp))
                    }
                }
            }
        }
    }
}
