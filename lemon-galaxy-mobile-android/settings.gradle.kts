pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "LemonGalaxy"
include(":app")

// Termux 终端仿真与终端控件（Apache-2.0），源码随仓库提供并改为远端字节流，见 vendor/termux/README.md
include(":terminal-emulator")
project(":terminal-emulator").projectDir = file("vendor/termux/terminal-emulator")
include(":terminal-view")
project(":terminal-view").projectDir = file("vendor/termux/terminal-view")
