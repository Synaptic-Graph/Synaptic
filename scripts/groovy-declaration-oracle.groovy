// Parse each original source through Groovy's CONVERSION phase, before type
// resolution or AST transforms. This works for independent build/test source
// sets without requiring their dependency classpaths. It does not build them.
// Usage: groovy groovy-declaration-oracle.groovy ROOT SOURCE_LIST OUTPUT
import groovy.json.JsonOutput
import org.codehaus.groovy.control.*

assert args.length == 3: 'ROOT SOURCE_LIST OUTPUT required'
def root = new File(args[0]).canonicalFile.toPath()
def results = []
new File(args[1]).readLines('UTF-8').findAll { it.trim() }.each { relative ->
    def file = root.resolve(relative).normalize().toFile().canonicalFile
    assert file.toPath().startsWith(root)
    try {
        def unit = new CompilationUnit()
        unit.addSource(file)
        unit.compile(Phases.CONVERSION)
        def declarations = []
        unit.AST.classes.each { c ->
            def name = c.nameWithoutPackage.tokenize('$').last()
            if (!c.script && !(c instanceof org.codehaus.groovy.ast.InnerClassNode && c.anonymous) && c.lineNumber > 0) {
                declarations << [name: name, line: c.lineNumber, column: c.columnNumber,
                    end: c.lastLineNumber, endColumn: c.lastColumnNumber, kind: 'type']
            }
            (c.methods + c.declaredConstructors).findAll {
                it.declaringClass == c && it.lineNumber > 0 && !it.synthetic && !it.scriptBody
            }.each { m ->
                declarations << [name: m.name == '<init>' ? name : m.name,
                    line: m.lineNumber, column: m.columnNumber,
                    end: m.lastLineNumber, endColumn: m.lastColumnNumber, kind: 'method']
            }
        }
        results << [file: relative, declarations: declarations]
    } catch (Exception error) {
        results << [file: relative, error: error.toString()]
    }
}
new File(args[2]).setText(JsonOutput.prettyPrint(JsonOutput.toJson([
    compiler: "Groovy ${GroovySystem.version}", phase: 'CONVERSION', files: results
])), 'UTF-8')
println "${results.size()} files, ${results.count { it.error }} compiler parsing failures"
