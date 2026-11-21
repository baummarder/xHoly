package runtime

/** JVM entry point for the xHoly Scala runtime bridge. */
object Main {
  def main(args: Array[String]): Unit = {
    val titleId = args.headOption.getOrElse("")
    if (titleId.nonEmpty) println(GameLoader.load(titleId))

    val in = scala.io.Source.stdin
    try {
      Iterator.continually(in.getLine())
        .takeWhile(_ != null)
        .foreach(line => println(GameLoader.handle(line)))
    } finally in.close()
  }
}
