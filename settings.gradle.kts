rootProject.name = "lib-net"

pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositories {
        mavenCentral()
    }
}

include(":version")
include(":launch-contract")
include(":net")
include(":report")
include(":sitecheck")
include(":trust")
include(":testpki")
