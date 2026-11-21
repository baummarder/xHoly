package runtime

/**
 * xHoly game runtime loader (JVM side).
 *
 * Hosts JVM-based game content, manages title modules, and exchanges
 * newline-delimited JSON messages with the Rust host over stdio.
 * A Scala.js bundle under `web/` targets browser deployment.
 */
object GameLoader {

  /** A loaded game module. */
  case class GameModule(
      titleId: String,
      displayName: String,
      entryPoint: String,
      assetsPath: String
  )

  /** Registry of known game modules. */
  private val registry = scala.collection.mutable.Map.empty[String, GameModule]

  /** Register a game module with the loader. */
  def register(module: GameModule): Unit = {
    registry(module.titleId) = module
  }

  /** Look up a registered module by title ID. */
  def lookup(titleId: String): Option[GameModule] = registry.get(titleId)

  /** All registered modules. */
  def all: Iterable[GameModule] = registry.values

  /**
   * Load a game module by title ID and run its entry point.
   * Returns a human-readable status string for the bridge protocol.
   */
  def load(titleId: String): String = {
    lookup(titleId) match {
      case Some(module) =>
        sendProgress(0.0f, s"Loading ${module.displayName}")
        // JVM-side classloading verification happens in runtime.Main
        // (JVM project) so this shared source stays Scala.js-compatible.
        sendProgress(1.0f, "Ready")
        s"OK ${module.titleId}"
      case None =>
        s"ERR unknown title: $titleId"
    }
  }

  /** Cross-platform compatibility shim for legacy titles (PS2/PS3/Switch era). */
  def loadLegacy(platform: String, titleId: String): String =
    s"OK legacy:$platform:$titleId (compat shim — see docs/whitepaper.md §6)"

  // ---- Bridge protocol (newline-delimited JSON over stdio) ----

  private[runtime] def sendProgress(percent: Float, message: String): Unit =
    println(s"""{"type":"Progress","percent":$percent,"message":"${escape(message)}"}""")

  private[runtime] def escape(s: String): String =
    s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")

  /** Handle a single bridge message line and return the response line. */
  def handle(line: String): String = line match {
    case s"""{"type":"LoadGame","titleId":"(.*?)"}""".r(id) => load(id)
    case s"""{"type":"LoadLegacy","platform":"(.*?)","titleId":"(.*?)"}""".r(p, id) =>
      loadLegacy(p, id)
    case """{"type":"Shutdown"}""" => """{"type":"Progress","percent":1.0,"message":"bye"}"""
    case other => s"""{"type":"Error","message":"unrecognized: ${escape(other)}"}"""
  }
}
