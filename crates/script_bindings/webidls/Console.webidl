[Exposed=*]
namespace console {
  undefined log(any... data);
  undefined info(any... data);
  undefined warn(any... data);
  undefined error(any... data);
  undefined debug(any... data);
  undefined dir(optional any item, optional object? options);
  undefined clear();
  undefined count(optional DOMString label = "default");
  undefined countReset(optional DOMString label = "default");
  undefined group(any... data);
  undefined groupCollapsed(any... data);
  undefined groupEnd();
  undefined time(optional DOMString label = "default");
  undefined timeEnd(optional DOMString label = "default");
  undefined timeLog(optional DOMString label = "default", any... data);
  undefined table(optional any tabularData, optional sequence<DOMString> properties);
  undefined trace(any... data);
  undefined assert(optional boolean condition = false, any... data);
};
