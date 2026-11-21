package cn.lemonit.galaxy.ui.ai

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import cn.lemonit.galaxy.ui.theme.Galaxy

/** 简单 Markdown：标题、段落、列表、代码块、行内代码、粗体。够显示模型回复用。 */
private sealed class MdBlock {
    data class Heading(val level: Int, val text: String) : MdBlock()
    data class Paragraph(val text: String) : MdBlock()
    data class Bullet(val marker: String, val text: String) : MdBlock()
    data class Code(val text: String) : MdBlock()
}

private fun parse(md: String): List<MdBlock> {
    val out = mutableListOf<MdBlock>()
    val lines = md.replace("\r\n", "\n").split('\n')
    var i = 0
    val para = StringBuilder()
    fun flush() {
        if (para.isNotBlank()) out.add(MdBlock.Paragraph(para.toString().trim()))
        para.clear()
    }
    while (i < lines.size) {
        val line = lines[i]
        val t = line.trimStart()
        when {
            t.startsWith("```") -> {
                flush()
                val code = StringBuilder()
                i++
                while (i < lines.size && !lines[i].trimStart().startsWith("```")) {
                    code.appendLine(lines[i])
                    i++
                }
                out.add(MdBlock.Code(code.toString().trimEnd()))
            }
            t.startsWith("#") -> {
                flush()
                val level = t.takeWhile { it == '#' }.length
                out.add(MdBlock.Heading(level, t.drop(level).trim()))
            }
            t.startsWith("- ") || t.startsWith("* ") -> {
                flush()
                out.add(MdBlock.Bullet("•", t.drop(2)))
            }
            Regex("^\\d+[.)] ").containsMatchIn(t) -> {
                flush()
                val m = Regex("^(\\d+)[.)] ").find(t)!!
                out.add(MdBlock.Bullet(m.groupValues[1] + ".", t.drop(m.value.length)))
            }
            t.isEmpty() -> flush()
            else -> {
                if (para.isNotEmpty()) para.append('\n')
                para.append(line)
            }
        }
        i++
    }
    flush()
    return out
}

@Composable
private fun inline(text: String): AnnotatedString {
    val codeBg = Galaxy.colors.codeBackground
    val codeFg = MaterialTheme.colorScheme.primary
    return buildAnnotatedString {
        var i = 0
        while (i < text.length) {
            when {
                text.startsWith("**", i) -> {
                    val end = text.indexOf("**", i + 2)
                    if (end > 0) {
                        withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(text.substring(i + 2, end)) }
                        i = end + 2
                    } else {
                        append(text[i]); i++
                    }
                }
                text[i] == '`' -> {
                    val end = text.indexOf('`', i + 1)
                    if (end > 0) {
                        withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeBg, color = codeFg)) {
                            append(text.substring(i + 1, end))
                        }
                        i = end + 1
                    } else {
                        append(text[i]); i++
                    }
                }
                else -> {
                    append(text[i]); i++
                }
            }
        }
    }
}

@Composable
fun MarkdownText(md: String, modifier: Modifier = Modifier, color: Color = MaterialTheme.colorScheme.onSurface) {
    val blocks = parse(md)
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        blocks.forEach { b ->
            when (b) {
                is MdBlock.Heading -> Text(
                    inline(b.text),
                    style = if (b.level <= 2) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleSmall,
                    color = color,
                )
                is MdBlock.Paragraph -> Text(inline(b.text), style = MaterialTheme.typography.bodyMedium, color = color)
                is MdBlock.Bullet -> Row {
                    Text(b.marker, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.width(20.dp))
                    Text(inline(b.text), style = MaterialTheme.typography.bodyMedium, color = color)
                }
                is MdBlock.Code -> Text(
                    b.text,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = color,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(8.dp))
                        .background(Galaxy.colors.codeBackground)
                        .horizontalScroll(rememberScrollState())
                        .padding(10.dp),
                )
            }
        }
    }
}
