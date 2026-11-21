package cn.lemonit.galaxy.core.mock

import cn.lemonit.galaxy.core.bridge.RecordingInfo
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.io.File

/**
 * 模拟录像：把推给界面的 H.264 访问单元原样收下，停止时写成一个普通 MP4（只有视频轨，不重新编码）。
 * 纯 Kotlin 实现，JVM 单元测试与真机上都能用；样本先放内存（模拟画面只有几十 KB 一帧，够用）。
 */
internal class MockRecorder(private val file: File, private val width: Int, private val height: Int) {
    private class Sample(val data: ByteArray, val key: Boolean, val ptsUs: Long)

    private val samples = mutableListOf<Sample>()
    private var sps: ByteArray? = null
    private var pps: ByteArray? = null
    private val startedAt = System.currentTimeMillis()

    init {
        file.parentFile?.mkdirs()
        // 先建出文件（和真内核一样边录边写，开始后就能看到）
        file.writeBytes(ByteArray(0))
    }

    /** 从第一个带 SPS/PPS 的关键帧开始收。 */
    @Synchronized
    fun push(annexB: ByteArray, ptsUs: Long, keyframe: Boolean) {
        val nals = splitNals(annexB)
        nals.forEach { n ->
            when (n[0].toInt() and 0x1F) {
                7 -> if (sps == null) sps = n
                8 -> if (pps == null) pps = n
            }
        }
        if (samples.isEmpty() && (!keyframe || sps == null || pps == null)) return
        // 样本里只留图像数据（AUD、参数集放进 avcC）
        val body = ByteArrayOutputStream()
        nals.filter { (it[0].toInt() and 0x1F) !in setOf(7, 8, 9) }.forEach { n ->
            body.write(int32(n.size))
            body.write(n)
        }
        if (body.size() > 0) samples.add(Sample(body.toByteArray(), keyframe, ptsUs))
    }

    @Synchronized
    fun finish(): RecordingInfo {
        val ms = System.currentTimeMillis() - startedAt
        if (samples.isEmpty() || sps == null || pps == null) {
            file.delete()
            return RecordingInfo(emptyList(), ms.toULong(), 0u)
        }
        writeMp4()
        return RecordingInfo(listOf(file.path), ms.toULong(), file.length().toULong())
    }

    // ---------------- MP4 ----------------

    private fun writeMp4() {
        val n = samples.size
        // 每帧时长（毫秒）：按相邻时间戳，最后一帧沿用前一帧
        val deltas = IntArray(n) { i ->
            if (i + 1 < n) ((samples[i + 1].ptsUs - samples[i].ptsUs) / 1000).toInt().coerceAtLeast(1) else 0
        }
        if (n > 1) deltas[n - 1] = deltas[n - 2] else deltas[0] = 33
        val duration = deltas.sum()
        val ftyp = box("ftyp") {
            writeBytes("isom")
            writeInt(0x200)
            listOf("isom", "iso2", "avc1", "mp41").forEach { writeBytes(it) }
        }
        val mdatSize = 8L + samples.sumOf { it.data.size.toLong() }
        val chunkOffset = ftyp.size + 8
        val moov = moov(deltas, duration, chunkOffset.toLong())
        file.outputStream().buffered().use { out ->
            out.write(ftyp)
            out.write(int32(mdatSize.toInt()))
            out.write("mdat".toByteArray())
            samples.forEach { out.write(it.data) }
            out.write(moov)
        }
    }

    private fun moov(deltas: IntArray, duration: Int, chunkOffset: Long): ByteArray = box("moov") {
        write(box("mvhd") {
            fullHeader(0, 0)
            writeInt(0); writeInt(0)
            writeInt(TIMESCALE); writeInt(duration)
            writeInt(0x00010000); writeShort(0x0100); write(ByteArray(10))
            matrix()
            write(ByteArray(24))
            writeInt(2)
        })
        write(box("trak") {
            write(box("tkhd") {
                fullHeader(0, 3)
                writeInt(0); writeInt(0)
                writeInt(1); writeInt(0); writeInt(duration)
                write(ByteArray(8))
                writeShort(0); writeShort(0); writeShort(0); writeShort(0)
                matrix()
                writeInt(width shl 16); writeInt(height shl 16)
            })
            write(box("mdia") {
                write(box("mdhd") {
                    fullHeader(0, 0)
                    writeInt(0); writeInt(0)
                    writeInt(TIMESCALE); writeInt(duration)
                    writeShort(0x55C4) // und
                    writeShort(0)
                })
                write(box("hdlr") {
                    fullHeader(0, 0)
                    writeInt(0); writeBytes("vide"); write(ByteArray(12))
                    writeBytes("VideoHandler"); writeByte(0)
                })
                write(box("minf") {
                    write(box("vmhd") { fullHeader(0, 1); write(ByteArray(8)) })
                    write(box("dinf") {
                        write(box("dref") {
                            fullHeader(0, 0); writeInt(1)
                            write(box("url ") { fullHeader(0, 1) })
                        })
                    })
                    write(stbl(deltas, chunkOffset))
                })
            })
        })
    }

    private fun stbl(deltas: IntArray, chunkOffset: Long): ByteArray = box("stbl") {
        val s = sps!!
        val p = pps!!
        write(box("stsd") {
            fullHeader(0, 0); writeInt(1)
            write(box("avc1") {
                write(ByteArray(6)); writeShort(1)
                write(ByteArray(16))
                writeShort(width); writeShort(height)
                writeInt(0x00480000); writeInt(0x00480000)
                writeInt(0); writeShort(1)
                write(ByteArray(32))
                writeShort(0x0018); writeShort(-1)
                write(box("avcC") {
                    writeByte(1); writeByte(s[1].toInt()); writeByte(s[2].toInt()); writeByte(s[3].toInt())
                    writeByte(0xFF); writeByte(0xE1)
                    writeShort(s.size); write(s)
                    writeByte(1); writeShort(p.size); write(p)
                })
            })
        })
        // 时长相同的相邻帧合并成一项
        val runs = mutableListOf<Pair<Int, Int>>()
        deltas.forEach { d -> if (runs.isNotEmpty() && runs.last().second == d) runs[runs.lastIndex] = runs.last().first + 1 to d else runs.add(1 to d) }
        write(box("stts") {
            fullHeader(0, 0); writeInt(runs.size)
            runs.forEach { (count, d) -> writeInt(count); writeInt(d) }
        })
        val keys = samples.indices.filter { samples[it].key }
        write(box("stss") {
            fullHeader(0, 0); writeInt(keys.size)
            keys.forEach { writeInt(it + 1) }
        })
        write(box("stsc") {
            fullHeader(0, 0); writeInt(1)
            writeInt(1); writeInt(samples.size); writeInt(1)
        })
        write(box("stsz") {
            fullHeader(0, 0); writeInt(0); writeInt(samples.size)
            samples.forEach { writeInt(it.data.size) }
        })
        write(box("stco") {
            fullHeader(0, 0); writeInt(1); writeInt(chunkOffset.toInt())
        })
    }

    companion object {
        private const val TIMESCALE = 1000

        private fun int32(v: Int) = byteArrayOf((v ushr 24).toByte(), (v ushr 16).toByte(), (v ushr 8).toByte(), v.toByte())

        private fun box(type: String, body: DataOutputStream.() -> Unit): ByteArray {
            val buf = ByteArrayOutputStream()
            DataOutputStream(buf).apply(body).flush()
            val content = buf.toByteArray()
            return int32(content.size + 8) + type.toByteArray() + content
        }

        private fun DataOutputStream.fullHeader(version: Int, flags: Int) = writeInt((version shl 24) or flags)

        private fun DataOutputStream.matrix() = listOf(0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000).forEach { writeInt(it) }

        /** Annex B → 去掉起始码的 NAL 列表。 */
        fun splitNals(b: ByteArray): List<ByteArray> {
            val starts = mutableListOf<Pair<Int, Int>>() // (起始码位置, 数据开始)
            var i = 0
            while (i + 2 < b.size) {
                if (b[i] == 0.toByte() && b[i + 1] == 0.toByte()) {
                    if (b[i + 2] == 1.toByte()) {
                        starts.add(i to i + 3); i += 3; continue
                    }
                    if (i + 3 < b.size && b[i + 2] == 0.toByte() && b[i + 3] == 1.toByte()) {
                        starts.add(i to i + 4); i += 4; continue
                    }
                }
                i++
            }
            return starts.mapIndexedNotNull { idx, (_, from) ->
                val to = if (idx + 1 < starts.size) starts[idx + 1].first else b.size
                if (to > from) b.copyOfRange(from, to) else null
            }
        }
    }
}
