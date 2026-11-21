import java.net.URI
import java.util.Base64
import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val ndkVersionName = "28.2.13676358"
val minSdkLevel = 26

// Rust 内核在仓库根的 cargo workspace 里：
//   lemon-galaxy/
//     Cargo.toml                                 workspace 根
//     lemon-galaxy-common-rust/galaxy-ffi/       产出 libgalaxy_ffi.so（uniffi 0.29）
//     lemon-galaxy-mobile-android/               本工程
// 默认（gradle.properties）：编译真内核，界面接真内核。
//   -Pgalaxy.skipRust=true   跳过内核编译与绑定生成（只改界面、本机没有 Rust 工具链时用；此时自动改用模拟内核）
//   -Pgalaxy.core=mock       编译内核但界面仍走模拟实现（截图测试始终用模拟实现，JVM 上加载不了安卓 .so）
val localProps = Properties().also { p ->
    rootProject.file("local.properties").takeIf { it.isFile }?.reader()?.use { r -> p.load(r) }
}
val sdkDir = localProps.getProperty("sdk.dir") ?: System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
val repoRoot = rootProject.projectDir.resolve("..").normalize()
val skipRust = (findProperty("galaxy.skipRust") as String?)?.toBoolean() ?: false
val coreImpl = if (skipRust) "mock" else (findProperty("galaxy.core") as String?) ?: "rust"
val rustJniLibs = layout.buildDirectory.dir("generated/rust-jniLibs")
// uniffi 生成的 Kotlin 绑定：每次随 .so 重新生成，放在构建目录（不进 git，保证与 .so 一致）
val uniffiKotlinDir = layout.buildDirectory.dir("generated/source/uniffi/kotlin")

// ABI → Rust target → NDK clang 前缀
data class RustAbi(val abi: String, val target: String, val clang: String)
val rustAbis = listOf(
    RustAbi("arm64-v8a", "aarch64-linux-android", "aarch64-linux-android$minSdkLevel-clang"),
    RustAbi("armeabi-v7a", "armv7-linux-androideabi", "armv7a-linux-androideabi$minSdkLevel-clang"),
    RustAbi("x86_64", "x86_64-linux-android", "x86_64-linux-android$minSdkLevel-clang"),
)

val ndkDir: File by lazy { file("$sdkDir/ndk/$ndkVersionName") }
val ndkBin: File by lazy {
    val hostTag = when {
        org.gradle.internal.os.OperatingSystem.current().isMacOsX -> "darwin-x86_64"
        org.gradle.internal.os.OperatingSystem.current().isLinux -> "linux-x86_64"
        else -> "windows-x86_64"
    }
    ndkDir.resolve("toolchains/llvm/prebuilt/$hostTag/bin")
}
// Android Studio 启动的 Gradle 守护进程 PATH 里常常没有 ~/.cargo/bin
val cargo = System.getenv("CARGO")
    ?: file("${System.getProperty("user.home")}/.cargo/bin/cargo").takeIf { it.isFile }?.path
    ?: "cargo"
val exeSuffix = if (org.gradle.internal.os.OperatingSystem.current().isWindows) ".exe" else ""

fun Exec.checkRustEnv() {
    doFirst {
        require(repoRoot.resolve("Cargo.toml").isFile) { "找不到内核工程 $repoRoot/Cargo.toml（或用 -Pgalaxy.skipRust=true 跳过）" }
        requireNotNull(sdkDir) { "local.properties 里没有 sdk.dir，也没有 ANDROID_HOME" }
        require(ndkBin.isDirectory) { "找不到 NDK $ndkVersionName：$ndkBin" }
    }
}

val buildRustTasks = rustAbis.map { a ->
    val envKey = a.target.uppercase().replace('-', '_')
    val ccKey = a.target.replace('-', '_')
    tasks.register<Exec>("buildRustCore_${a.abi.replace('-', '_')}") {
        description = "cargo build -p galaxy-ffi --release --target ${a.target} --lib"
        group = "galaxy"
        onlyIf { !skipRust }
        // cargo 自己做增量，每次都交给它判断
        outputs.upToDateWhen { false }
        workingDir = repoRoot
        checkRustEnv()
        commandLine(cargo, "build", "-p", "galaxy-ffi", "--release", "--target", a.target, "--lib")
        environment("CARGO_TARGET_${envKey}_LINKER", ndkBin.resolve(a.clang).path)
        environment("CC_$ccKey", ndkBin.resolve(a.clang).path)
        environment("CXX_$ccKey", ndkBin.resolve(a.clang + "++").path)
        environment("AR_$ccKey", ndkBin.resolve("llvm-ar").path)
        environment("ANDROID_NDK_HOME", ndkDir.path)
        environment("ANDROID_NDK_ROOT", ndkDir.path)
    }
}

val copyRustCore by tasks.registering(Copy::class) {
    description = "Copy libgalaxy_ffi.so of every ABI into generated jniLibs"
    group = "galaxy"
    onlyIf { !skipRust }
    dependsOn(buildRustTasks)
    rustAbis.forEach { a ->
        from(repoRoot.resolve("target/${a.target}/release/libgalaxy_ffi.so")) { into(a.abi) }
    }
    into(rustJniLibs)
}

// 绑定生成器是 galaxy-ffi 自带的 bin（需要 --features bindgen），在主机上编译
val buildUniffiBindgen by tasks.registering(Exec::class) {
    description = "cargo build -p galaxy-ffi --features bindgen --bin uniffi-bindgen"
    group = "galaxy"
    onlyIf { !skipRust }
    outputs.upToDateWhen { false }
    workingDir = repoRoot
    checkRustEnv()
    commandLine(cargo, "build", "-p", "galaxy-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen")
}

// 主机上编一份不剥符号的 galaxy_ffi 动态库，供绑定生成器读 uniffi 元数据
// （发布配置 strip = true，安卓 .so 里只剩动态符号，library 模式读不到元数据）
val hostLibName = when {
    org.gradle.internal.os.OperatingSystem.current().isMacOsX -> "libgalaxy_ffi.dylib"
    org.gradle.internal.os.OperatingSystem.current().isWindows -> "galaxy_ffi.dll"
    else -> "libgalaxy_ffi.so"
}
val buildHostCoreLib by tasks.registering(Exec::class) {
    description = "cargo build -p galaxy-ffi --lib (host, for uniffi metadata)"
    group = "galaxy"
    onlyIf { !skipRust }
    outputs.upToDateWhen { false }
    workingDir = repoRoot
    checkRustEnv()
    commandLine(cargo, "build", "-p", "galaxy-ffi", "--lib")
}

/**
 * uniffi 0.29 的 Kotlin 生成器有个已知问题：错误枚举的成员字段叫 `message` 时，会和 Throwable.message 冲突，生成的代码编译不过
 * （GalaxyError 的每个成员都有 message 字段）。这里只在 GalaxyException 及其转换器内把字段改名为 `reason`，
 * 并让 Throwable.message 直接返回它（内核已按 locale 本地化）。内核把字段改名后这段自动不生效。
 */
fun patchUniffiErrorMessageField(file: File) {
    val text = file.readText()
    val start = text.indexOf("sealed class GalaxyException")
    if (start < 0 || !text.substring(start).contains("override val message\n            get() = \"message=")) return
    val conv = text.indexOf("FfiConverterTypeGalaxyError", start)
    val end = text.indexOf("\n}\n", if (conv > 0) conv else start)
    require(end > start) { "GalaxyException 补丁：找不到转换器结尾" }
    var region = text.substring(start, end)
    region = region.replace("val `message`: kotlin.String", "val `reason`: kotlin.String")
        .replace(Regex("get\\(\\) = \"(code=\\$\\{ `code` \\}, )?message=\\$\\{ `message` \\}\""), "get() = `reason`")
        .replace("value.`message`", "value.`reason`")
    require(!region.contains("`message`")) { "GalaxyException 补丁不完整，检查 uniffi 生成代码是否变了" }
    file.writeText(text.substring(0, start) + region + text.substring(end))
}

// library 模式：从主机动态库读 uniffi 元数据生成 Kotlin（接口与安卓 .so 同源，校验和一致）
val generateUniffiKotlin by tasks.registering(Exec::class) {
    description = "Generate uniffi Kotlin bindings for galaxy-ffi into build/generated/source/uniffi/kotlin"
    group = "galaxy"
    onlyIf { !skipRust }
    dependsOn(buildUniffiBindgen, buildHostCoreLib)
    val lib = repoRoot.resolve("target/debug/$hostLibName")
    inputs.file(lib).withPropertyName("ffiLibrary")
    outputs.dir(uniffiKotlinDir)
    workingDir = repoRoot
    doFirst { uniffiKotlinDir.get().asFile.deleteRecursively() }
    doLast {
        val kt = uniffiKotlinDir.get().asFile.resolve("uniffi/galaxy_ffi/galaxy_ffi.kt")
        require(kt.isFile) { "uniffi-bindgen 没有生成 galaxy_ffi.kt：检查 $lib 是否带 uniffi 元数据" }
        patchUniffiErrorMessageField(kt)
    }
    commandLine(
        repoRoot.resolve("target/debug/uniffi-bindgen$exeSuffix").path,
        "generate", "--library", lib.path,
        "--language", "kotlin", "--out-dir", uniffiKotlinDir.get().asFile.path, "--no-format",
    )
}

// 正式签名，优先级：环境变量（CI）> keystore/keystore.properties（本机，不进 git）> 调试签名
//   CI：LEMONGALAXY_KEYSTORE_BASE64（jks 的 base64）、LEMONGALAXY_KEYSTORE_PASSWORD、LEMONGALAXY_KEY_ALIAS、LEMONGALAXY_KEY_PASSWORD
//   本机：keystore/keystore.properties 里 storeFile（相对工程根）、storePassword、keyAlias、keyPassword
data class ReleaseKey(val file: File, val storePassword: String, val alias: String, val keyPassword: String)
val releaseKey: ReleaseKey? = run {
    val b64 = System.getenv("LEMONGALAXY_KEYSTORE_BASE64")?.trim()?.takeIf { it.isNotEmpty() }
    if (b64 != null) {
        val out = layout.buildDirectory.file("tmp/signing/release.jks").get().asFile
        out.parentFile.mkdirs()
        out.writeBytes(Base64.getMimeDecoder().decode(b64))
        val storePw = System.getenv("LEMONGALAXY_KEYSTORE_PASSWORD").orEmpty()
        return@run ReleaseKey(
            out, storePw,
            System.getenv("LEMONGALAXY_KEY_ALIAS").orEmpty(),
            System.getenv("LEMONGALAXY_KEY_PASSWORD")?.takeIf { it.isNotEmpty() } ?: storePw,
        )
    }
    val props = Properties().also { p ->
        rootProject.file("keystore/keystore.properties").takeIf { it.isFile }?.reader()?.use { r -> p.load(r) }
    }
    val storeFile = props.getProperty("storeFile") ?: return@run null
    ReleaseKey(
        rootProject.file(storeFile), props.getProperty("storePassword").orEmpty(),
        props.getProperty("keyAlias").orEmpty(), props.getProperty("keyPassword").orEmpty(),
    )
}

android {
    namespace = "cn.lemonit.galaxy"
    compileSdk = 36
    ndkVersion = ndkVersionName

    signingConfigs {
        if (releaseKey != null) {
            create("release") {
                storeFile = releaseKey.file
                storePassword = releaseKey.storePassword
                keyAlias = releaseKey.alias
                keyPassword = releaseKey.keyPassword
            }
        }
    }

    defaultConfig {
        applicationId = "cn.lemonit.galaxy"
        minSdk = minSdkLevel
        targetSdk = 36
        // 云端构建传入：版本号取标签或 0.1.0，versionCode 取构建序号（每次递增，手机才能覆盖升级）
        versionCode = System.getenv("LEMONGALAXY_VERSION_CODE")?.toIntOrNull() ?: 1
        versionName = System.getenv("LEMONGALAXY_VERSION_NAME")?.takeIf { it.isNotBlank() } ?: "0.1.0"
        ndk { abiFilters += rustAbis.map { it.abi } }
        buildConfigField("String", "CORE_IMPL", "\"$coreImpl\"")
        buildConfigField("boolean", "USE_MOCK_CORE", (coreImpl != "rust").toString())
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    if (!skipRust) {
        sourceSets["main"].jniLibs.srcDir(rustJniLibs)
        // 真内核：uniffi 绑定 + 适配层（cn.lemonit.galaxy.core.rust.UniffiCore）
        sourceSets["main"].java.srcDir(generateUniffiKotlin.map { uniffiKotlinDir.get() })
        sourceSets["main"].java.srcDir("src/rust/java")
    }
    // 模拟内核用的假画面素材：调试包与单元测试（截图）总带上，发布包只在 core=mock 时带
    sourceSets["debug"].assets.srcDir("src/mock/assets")
    if (coreImpl == "mock") sourceSets["release"].assets.srcDir("src/mock/assets")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    androidResources {
        // 应用内切换语言（Android 13+ 系统设置里也能按应用选语言）
        generateLocaleConfig = true
    }

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "/META-INF/versions/9/OSGI-INF/MANIFEST.MF")
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
        // 截图测试（Robolectric + Roborazzi）要读资源与 assets（终端配色、模拟画面）
        unitTests.isIncludeAndroidResources = true
        unitTests.all { test ->
            // 截图测试每次都重新生成 PNG（app/build/outputs/roborazzi/）；-Proborazzi.record=false 时跳过
            test.systemProperty("roborazzi.test.record", findProperty("roborazzi.record")?.toString() ?: "true")
            // Robolectric 运行时可能要下载 android-all；本机走代理时把 HTTPS_PROXY 交给测试进程
            System.getenv("HTTPS_PROXY")?.let { URI(it) }?.takeIf { it.host != null && it.port > 0 }?.let { u ->
                test.systemProperty("https.proxyHost", u.host)
                test.systemProperty("https.proxyPort", u.port.toString())
            }
            test.maxHeapSize = "3g"
            // 真内核冒烟测试（UniffiCoreSmokeTest）：在主机 JVM 上用 JNA 加载主机版 galaxy_ffi 动态库
            test.systemProperty("galaxy.hostLibDir", repoRoot.resolve("target/debug").path)
            test.systemProperty("galaxy.skipRust", skipRust.toString())
        }
    }

    lint {
        abortOnError = false
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
        freeCompilerArgs.add("-opt-in=kotlin.ExperimentalUnsignedTypes")
    }
}

tasks.named("preBuild") {
    dependsOn(copyRustCore)
    if (!skipRust) dependsOn(generateUniffiKotlin)
}

tasks.withType<Test>().configureEach {
    if (!skipRust) dependsOn(buildHostCoreLib)
    // 失败时打出完整异常
    testLogging {
        events("failed")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showCauses = true
        showStackTraces = true
    }
}

dependencies {
    implementation(project(":terminal-view"))

    val composeBom = platform("androidx.compose:compose-bom:2025.04.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    debugImplementation("androidx.compose.ui:ui-tooling")

    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-process:2.8.7")
    implementation("androidx.biometric:biometric:1.1.0")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")

    // uniffi 生成的 Kotlin 绑定通过 JNA 调用 libgalaxy_ffi.so
    implementation("net.java.dev.jna:jna:5.19.1@aar")

    // 扫码填服务器地址（不依赖 Google Play 服务，国内手机也能用）
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    implementation("com.google.zxing:core:3.5.3")

    testImplementation("junit:junit:4.13.2")
    // 主机 JVM 上跑真内核冒烟测试要用带桌面本地库的 JNA（安卓用的 @aar 只有安卓 .so）
    testImplementation("net.java.dev.jna:jna:5.19.1")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")

    // 界面截图测试：JVM 上用 Robolectric 渲染 Compose，Roborazzi 存 PNG
    testImplementation(composeBom)
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.43.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.43.0")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-junit-rule:1.43.0")
}
