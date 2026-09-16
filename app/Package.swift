// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "cpx",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "CpxKit", targets: ["CpxKit"]),
        .executable(name: "cpx-app", targets: ["CpxApp"]),
    ],
    targets: [
        // Everything that can be reasoned about without a window: the
        // contract with the `cpx` command, the refresh policy, what makes a
        // notification fire, provider status feeds.
        .target(name: "CpxKit", path: "Sources/CpxKit"),
        .executableTarget(
            name: "CpxApp",
            dependencies: ["CpxKit"],
            path: "Sources/CpxApp",
            resources: [.copy("Resources/cpx.icns")]
        ),
        .testTarget(name: "CpxKitTests", dependencies: ["CpxKit"], path: "Tests/CpxKitTests"),
    ],
    swiftLanguageVersions: [.v5]
)
