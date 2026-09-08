// Run with the project's Groovy compiler/classpath. Compiles through class
// generation (including AST transforms), but does not run application methods.
// Usage: groovy export-groovy-facts.groovy ROOT OUTPUT SOURCE_LIST [CLASSPATH]
import groovy.json.JsonOutput
import org.codehaus.groovy.control.*
import org.codehaus.groovy.ast.CodeVisitorSupport
import org.codehaus.groovy.ast.expr.*

assert args.length >= 3: 'ROOT OUTPUT SOURCE_LIST [CLASSPATH] required'
def root = new File(args[0]).canonicalFile.toPath()
def config = new CompilerConfiguration()
if (args.length > 3) config.classpath = args[3]
def unit = new CompilationUnit(config)
def files = new File(args[2]).readLines('UTF-8').findAll { it.trim() }.collect {
    def file = root.resolve(it).normalize().toFile().canonicalFile
    assert file.toPath().startsWith(root): "source outside root: $file"
    file
}
files.each { unit.addSource(it) }
def symbol = { method ->
    "${method.declaringClass.name}#${method.name}(${method.parameters.collect { it.type.name }.join(',')})".toString()
}
unit.compile(Phases.CONVERSION)
def original = [:]
unit.AST.classes.each { c ->
    (c.methods + c.declaredConstructors).each { m -> original[symbol(m)] = m.lineNumber }
}
unit.compile(Phases.CLASS_GENERATION)
def facts = files.collectEntries { file ->
    [(root.relativize(file.toPath()).toString().replace('\\', '/')):
        [source: file.getText('UTF-8'), methods: []]]
}
unit.AST.classes.each { c ->
    def file = new File(c.module.context.name).canonicalFile.toPath()
    def item = facts[root.relativize(file).toString().replace('\\', '/')]
    if (item == null) return
    (c.methods + c.declaredConstructors).findAll { it.declaringClass == c }.each { m ->
        def id = symbol(m)
        def generated = !original.containsKey(id)
        if (generated && (m.name.startsWith('$') || m.name in ['getMetaClass', 'setMetaClass'])) return
        def calls = []
        if (m.code != null) m.code.visit(new CodeVisitorSupport() {
            @Override void visitMethodCallExpression(MethodCallExpression call) {
                def target = call.methodTarget
                calls << [line: Math.max(1, call.lineNumber),
                    target: target == null ? null : symbol(target),
                    name: call.methodAsString, snippet: call.text]
                super.visitMethodCallExpression(call)
            }
            @Override void visitStaticMethodCallExpression(StaticMethodCallExpression call) {
                // Only explicit MethodNode targets count as resolved evidence.
                def target = call.getNodeMetaData(org.codehaus.groovy.transform.stc.StaticTypesMarker.DIRECT_METHOD_CALL_TARGET)
                calls << [line: Math.max(1, call.lineNumber),
                    target: target == null ? null : symbol(target),
                    name: call.method, snippet: call.text]
                super.visitStaticMethodCallExpression(call)
            }
        })
        item.methods << [symbol: id, owner: c.nameWithoutPackage,
            name: m.name == '<init>' ? c.nameWithoutPackage : m.name,
            line: Math.max(1, original[id] ?: c.lineNumber), generated: generated,
            parameters: m.parameters.collect { [name: it.name, type_ref: it.type.name] },
            calls: calls]
    }
}
def output = new File(args[1])
output.parentFile?.mkdirs()
output.setText(JsonOutput.prettyPrint(JsonOutput.toJson([
    version: 1, compiler: "Groovy ${GroovySystem.version}", files: facts,
    classpath: config.classpath.collectMany { p ->
        def f = new File(p).canonicalFile
        def inputs = [f]
        if (f.isDirectory()) f.eachFileRecurse { inputs << it.canonicalFile }
        inputs.collect { input -> [path: input.path, directory: input.isDirectory(), size: input.length(), modified: input.lastModified()] }
    }
])), 'UTF-8')
println "Exported ${facts.size()} sources, ${facts.values().sum { it.methods.size() }} methods to $output"
