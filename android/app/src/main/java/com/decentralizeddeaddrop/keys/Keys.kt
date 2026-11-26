package com.decentralizeddeaddrop.keys

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.decentralizeddeaddrop.engine.KeyHolder
import java.io.File
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

private const val PROVIDER = "AndroidKeyStore"

private fun keyStore(): KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

/**
 * One identity's two long-term keys (docs/PROTOCOL.md section 4), made in the Android
 * Keystore on first use. The private halves never leave the phone's secure hardware:
 * this class, and the engine behind it, can only ask for them to be used.
 *
 * Each server gets its own identity ([slot]), so two servers cannot tell that the same
 * phone uses both.
 */
class DeviceKeys(private val slot: Int) : KeyHolder {
    private val signingAlias = "deaddrop.$slot.signing"
    private val agreementAlias = "deaddrop.$slot.agreement"
    private val store = keyStore()

    init {
        ensure(signingAlias, KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY) {
            setDigests(KeyProperties.DIGEST_SHA256)
        }
        ensure(agreementAlias, KeyProperties.PURPOSE_AGREE_KEY) {}
    }

    override fun signingKeyDer(): ByteArray = store.getCertificate(signingAlias).publicKey.encoded

    override fun agreementKeyDer(): ByteArray = store.getCertificate(agreementAlias).publicKey.encoded

    /** The signing key vouches for the agreement key, so a server cannot swap in its own. */
    override fun agreementSignature(): String {
        val agreement = Base64.encodeToString(agreementKeyDer(), Base64.NO_WRAP)
        val signature = sign("deaddrop/agreement-key/v1\n$agreement".toByteArray())
        return Base64.encodeToString(signature, Base64.NO_WRAP)
    }

    override fun sign(message: ByteArray): ByteArray =
        Signature.getInstance("SHA256withECDSA").run {
            initSign(store.getKey(signingAlias, null) as PrivateKey)
            update(message)
            sign()
        }

    /** Destroys this identity. Nothing signed with it can ever be signed again. */
    fun destroy() {
        store.deleteEntry(signingAlias)
        store.deleteEntry(agreementAlias)
    }

    private fun ensure(alias: String, purposes: Int, configure: KeyGenParameterSpec.Builder.() -> Unit) {
        if (store.containsAlias(alias)) return
        val spec = KeyGenParameterSpec.Builder(alias, purposes)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .apply(configure)
            .build()
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, PROVIDER).apply {
            initialize(spec)
            generateKeyPair()
        }
    }
}

/**
 * Files on the phone's storage, encrypted with a key that never leaves the Keystore
 * (AES-256-GCM, a fresh nonce for every write). Holds the engine's state, which is
 * secret key material, and the message history.
 */
class Vault(private val directory: File) {
    private val key: SecretKey = keyStore().let { store ->
        (store.getKey(ALIAS, null) as SecretKey?) ?: KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER).run {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
            generateKey()
        }
    }

    init {
        directory.mkdirs()
    }

    /** Replaces a file's contents. The old contents stay until the new are fully written. */
    @Synchronized
    fun write(name: String, contents: ByteArray) {
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key) }
        val sealed = cipher.iv + cipher.doFinal(contents)
        val temporary = File(directory, "$name.writing")
        temporary.outputStream().use { out ->
            out.write(sealed)
            out.fd.sync()
        }
        if (!temporary.renameTo(File(directory, name))) error("cannot store $name")
    }

    /** A file's contents, or null if it does not exist or cannot be opened. */
    @Synchronized
    fun read(name: String): ByteArray? {
        val file = File(directory, name)
        if (!file.exists()) return null
        return runCatching {
            val sealed = file.readBytes()
            val cipher = Cipher.getInstance(TRANSFORMATION).apply {
                init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, sealed, 0, NONCE_BYTES))
            }
            cipher.doFinal(sealed, NONCE_BYTES, sealed.size - NONCE_BYTES)
        }.getOrNull()
    }

    @Synchronized
    fun delete(name: String) {
        File(directory, name).delete()
        File(directory, "$name.writing").delete()
    }

    private companion object {
        const val ALIAS = "deaddrop.storage"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val NONCE_BYTES = 12
    }
}
