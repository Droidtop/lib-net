plugins {
    kotlin("jvm") version "2.2.21" apply false
}

// Every module is plain JVM (java.security and the standard library only), so its tests run
// on a build machine with no device and no Android SDK.
subprojects {
    group = "dev.droidtop.shared"
    version = rootProject.property("lib.version") as String
}
