// Root sbt build for xHoly.
//
// The authoritative sbt build lives in `scala/build.sbt` (crossProject:
// JVM runtime + Scala.js web bundle). This root file exists so `sbt`
// invoked at the repository root still resolves a valid build and can
// aggregate documentation/benchmark helpers.

ThisBuild / scalaVersion := "3.3.4"

lazy val root = (project in file("."))
  .settings(
    name := "xholy",
    publish / skip := true
  )
