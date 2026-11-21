package cn.lemonit.galaxy.core.platform

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import cn.lemonit.galaxy.core.bridge.SecureStore
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * ffi.md 1.1 的 SecureStore：Android Keystore 里生成不可导出的 AES-256 密钥，
 * 每个键值用 AES-GCM 加密后存成应用私有目录下的一个文件（文件名是键的 SHA-256，键名本身不落盘）。
 *
 * 文件格式：版本 1 字节 | IV 12 字节 | 密文+标签。关联数据为键名，防止文件被互相替换。
 */
class AndroidSecureStore(context: Context) : SecureStore {
    private val dir = File(context.noBackupFilesDir, "secure").apply { mkdirs() }
    private val lock = Any()

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    private fun fileFor(key: String): File {
        val h = MessageDigest.getInstance("SHA-256").digest(key.toByteArray())
        return File(dir, h.joinToString("") { "%02x".format(it) })
    }

    override fun get(key: String): ByteArray? = synchronized(lock) {
        val f = fileFor(key)
        if (!f.isFile) return null
        return try {
            val raw = f.readBytes()
            if (raw.size < 1 + IV_LEN || raw[0] != VERSION) return null
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw, 1, IV_LEN))
            cipher.updateAAD(key.toByteArray())
            cipher.doFinal(raw, 1 + IV_LEN, raw.size - 1 - IV_LEN)
        } catch (e: Exception) {
            // 密钥被系统清掉（例如锁屏方式变更）或文件损坏：当作不存在
            Log.w(TAG, "decrypt failed for an entry: ${e.javaClass.simpleName}")
            null
        }
    }

    override fun set(key: String, value: ByteArray) = synchronized(lock) {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.updateAAD(key.toByteArray())
        val ct = cipher.doFinal(value)
        val iv = cipher.iv
        check(iv.size == IV_LEN)
        val tmp = File(dir, fileFor(key).name + ".tmp")
        tmp.writeBytes(byteArrayOf(VERSION) + iv + ct)
        if (!tmp.renameTo(fileFor(key))) {
            fileFor(key).delete()
            tmp.renameTo(fileFor(key))
        }
        Unit
    }

    override fun remove(key: String) = synchronized(lock) {
        fileFor(key).delete()
        Unit
    }

    private companion object {
        const val TAG = "SecureStore"
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "lemongalaxy.securestore.v1"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
        const val VERSION: Byte = 1
    }
}

/** 截图测试 / 没有 Keystore 的环境用的内存实现。 */
class MemorySecureStore : SecureStore {
    private val map = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
    override fun get(key: String): ByteArray? = map[key]
    override fun set(key: String, value: ByteArray) {
        map[key] = value
    }

    override fun remove(key: String) {
        map.remove(key)
    }
}
