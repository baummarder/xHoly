// xHoly JVM + Scala.js runtime bridge.
// Run `sbt` from the repo root for the Rust core (Cargo), or `cd scala && sbt` for this build.

ThisBuild / scalaVersion := "3.3.4"
ThisBuild / organization := "dev.xholy"
ThisBuild / version := "0.1.0"

lazy val scalaVer = "3.3.4"
lazy val scalajsVer = "1.16.0"

lazy val runtime = crossProject(JSPlatform, JVMPlatform)
  .crossType(CrossType.Pure)
  .in(file("."))
  .settings(
    name := "xholy-runtime",
    libraryDependencies ++= Seq(
      "org.scala-lang.modules" %% "scala-collection-compat" % "2.11.0",
      "com.lihaoyi" %% "upickle" % "3.1.0",
      "com.lihaoyi" %%%% "upickle" % "3.1.0"
    )
  )
  .jvmSettings(
    Compile / mainClass := Some("runtime.Main")
  )
  .jsSettings(
    scalaJSUseMainModuleInitializer := true,
    Compile / fullOptJS / closureCompilerOptions +=
      com.google.javascript.jscomp.CompilationLevel.SIMPLE_OPTIMIZATIONS
  )

lazy val root = project
  .in(file("."))
  .aggregate(runtime.js, runtime.jvm)
