plugins {
    kotlin("jvm") version "2.2.21" apply false
    // Every module's public API is a checked-in dump (<module>/api/<module>.api); apiCheck fails on any difference
    // (Droidtop/tracker#867: stable library interfaces). scripts/check-api-compat.sh adds the version rule.
    id("org.jetbrains.kotlinx.binary-compatibility-validator") version "0.18.2"
}

apiValidation {
    // Throwaway certificate chains for the other modules' tests: not an interface any program uses.
    ignoredProjects += "testpki"
}

// Every module is plain JVM (java.security and the standard library only), so its tests run
// on a build machine with no device and no Android SDK.
subprojects {
    group = "dev.droidtop.shared"
    version = rootProject.property("lib.version") as String
}
